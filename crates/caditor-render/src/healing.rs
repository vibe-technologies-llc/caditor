#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceFailure {
    Validation,
    Lost,
    ReplacementLacksFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceRepair {
    Reconfigure,
    Recreate,
    ConfigureConservatively,
    ReplaceDevice,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SurfaceLadder {
    validation_failures: u32,
}

impl SurfaceLadder {
    pub(crate) fn climb(&mut self, failure: SurfaceFailure) -> SurfaceRepair {
        match failure {
            SurfaceFailure::Lost => SurfaceRepair::Recreate,
            SurfaceFailure::ReplacementLacksFormat => self.replace_device(),
            SurfaceFailure::Validation => {
                self.validation_failures = self.validation_failures.saturating_add(1);
                match self.validation_failures {
                    1 => SurfaceRepair::Reconfigure,
                    2 => SurfaceRepair::Recreate,
                    3 => SurfaceRepair::ConfigureConservatively,
                    _ => self.replace_device(),
                }
            }
        }
    }

    pub(crate) fn settle(&mut self) {
        *self = Self::default();
    }

    fn replace_device(&mut self) -> SurfaceRepair {
        self.settle();
        SurfaceRepair::ReplaceDevice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn climbed(ladder: &mut SurfaceLadder, failures: &[SurfaceFailure]) -> Vec<SurfaceRepair> {
        failures
            .iter()
            .map(|failure| ladder.climb(*failure))
            .collect()
    }

    #[test]
    fn validation_failures_climb_one_rung_each_up_to_a_new_device_and_start_again() {
        let mut ladder = SurfaceLadder::default();

        let repairs = climbed(&mut ladder, &[SurfaceFailure::Validation; 5]);

        assert_eq!(
            repairs,
            [
                SurfaceRepair::Reconfigure,
                SurfaceRepair::Recreate,
                SurfaceRepair::ConfigureConservatively,
                SurfaceRepair::ReplaceDevice,
                SurfaceRepair::Reconfigure,
            ]
        );
    }

    #[test]
    fn a_lost_surface_is_recreated_and_a_replacement_lacking_the_format_opens_a_new_device() {
        let mut ladder = SurfaceLadder::default();

        let repairs = climbed(
            &mut ladder,
            &[
                SurfaceFailure::Lost,
                SurfaceFailure::ReplacementLacksFormat,
                SurfaceFailure::Lost,
                SurfaceFailure::ReplacementLacksFormat,
            ],
        );

        assert_eq!(
            repairs,
            [
                SurfaceRepair::Recreate,
                SurfaceRepair::ReplaceDevice,
                SurfaceRepair::Recreate,
                SurfaceRepair::ReplaceDevice,
            ]
        );
        assert_eq!(ladder, SurfaceLadder::default());
    }

    #[test]
    fn a_new_device_or_a_good_frame_puts_the_ladder_back_on_its_first_rung() {
        let mut replaced = SurfaceLadder::default();
        let mut settled = SurfaceLadder::default();

        climbed(&mut replaced, &[SurfaceFailure::Validation; 2]);

        let after_replacement = climbed(
            &mut replaced,
            &[
                SurfaceFailure::ReplacementLacksFormat,
                SurfaceFailure::Validation,
            ],
        );

        climbed(&mut settled, &[SurfaceFailure::Validation; 3]);
        settled.settle();

        let after_settling = settled.climb(SurfaceFailure::Validation);

        assert_eq!(
            after_replacement,
            [SurfaceRepair::ReplaceDevice, SurfaceRepair::Reconfigure]
        );
        assert_eq!(after_settling, SurfaceRepair::Reconfigure);
    }

    #[test]
    fn a_lost_surface_between_validation_failures_keeps_their_count() {
        let mut ladder = SurfaceLadder::default();

        let repairs = climbed(
            &mut ladder,
            &[
                SurfaceFailure::Validation,
                SurfaceFailure::Lost,
                SurfaceFailure::Validation,
            ],
        );

        assert_eq!(
            repairs,
            [
                SurfaceRepair::Reconfigure,
                SurfaceRepair::Recreate,
                SurfaceRepair::Recreate,
            ]
        );
    }
}
