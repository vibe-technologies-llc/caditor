use caditor_render::View;
use caditor_sketch::Faceting;

const CHORD_PIXELS: f64 = 0.25;
const KEPT_LEVELS_ABOVE: i32 = 1;
const FINEST: i32 = -40;
const COARSEST: i32 = 40;
const WITHOUT_A_VIEW: i32 = -5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FacetLevel(i32);

impl FacetLevel {
    pub const WITHOUT_A_VIEW: Self = Self(WITHOUT_A_VIEW);

    pub fn wanted_chord(view: &View) -> f64 {
        CHORD_PIXELS * view.units_per_pixel_at(view.viewpoint().distance)
    }

    pub fn fitting(chord: f64) -> Self {
        if chord > 0.0 {
            Self((chord.log2().floor() as i32).clamp(FINEST, COARSEST))
        } else {
            Self(FINEST)
        }
    }

    pub fn following(kept: Option<Self>, chord: f64) -> Self {
        let fitting = Self::fitting(chord);
        match kept {
            Some(kept)
                if (kept.0..=kept.0.saturating_add(KEPT_LEVELS_ABOVE)).contains(&fitting.0) =>
            {
                kept
            }
            Some(_) | None => fitting,
        }
    }

    pub fn chord(self) -> f64 {
        2f64.powi(self.0)
    }

    pub fn faceting(self) -> Faceting {
        Faceting::within(self.chord())
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Point3, Vector3};
    use caditor_render::Viewpoint;

    use super::*;

    fn view_at(distance: f64) -> View {
        let viewpoint = Viewpoint::looking_from(Vector3::Z, Point3::ZERO, distance).unwrap();
        View::new(viewpoint, 1000.0, 800.0)
    }

    #[test]
    fn the_wanted_chord_is_a_quarter_pixel_at_the_views_target() {
        let view = view_at(200.0);
        let across = view.project(Point3::ZERO).unwrap();
        let beside = view
            .project(Point3::new(FacetLevel::wanted_chord(&view) * 4.0, 0.0, 0.0))
            .unwrap();

        assert!((beside.distance(across) - 1.0).abs() < 1e-6);
        assert!(
            (FacetLevel::wanted_chord(&view_at(400.0)) / FacetLevel::wanted_chord(&view) - 2.0)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn a_level_is_the_largest_power_of_two_not_above_the_wanted_chord() {
        for chord in [1e-5, 0.003, 0.25, 0.3, 1.0, 7.9, 4096.0] {
            let level = FacetLevel::fitting(chord);
            assert!(level.chord() <= chord);
            assert!(level.chord() * 2.0 > chord);
        }
        assert_eq!(FacetLevel::fitting(0.0), FacetLevel(FINEST));
        assert_eq!(FacetLevel::fitting(f64::NAN), FacetLevel(FINEST));
        assert_eq!(FacetLevel::fitting(f64::INFINITY), FacetLevel(COARSEST));
    }

    #[test]
    fn a_level_is_kept_until_the_wanted_chord_leaves_four_times_its_range() {
        let kept = FacetLevel::fitting(1.0);

        assert_eq!(FacetLevel::following(Some(kept), 1.0), kept);
        assert_eq!(FacetLevel::following(Some(kept), 3.9), kept);
        assert_eq!(
            FacetLevel::following(Some(kept), 4.0),
            FacetLevel::fitting(4.0)
        );
        assert_eq!(
            FacetLevel::following(Some(kept), 0.99),
            FacetLevel::fitting(0.99)
        );
        assert_eq!(FacetLevel::following(None, 3.0), FacetLevel::fitting(3.0));
    }

    fn changes_over(distances: impl Iterator<Item = f64>) -> u32 {
        let mut level = None;
        let mut changes = 0;
        for distance in distances {
            let next = FacetLevel::following(level, FacetLevel::wanted_chord(&view_at(distance)));
            if level.is_some_and(|level| level != next) {
                changes += 1;
            }
            level = Some(next);
        }
        changes
    }

    #[test]
    fn zooming_refacets_once_per_halving_in_once_per_two_doublings_out_and_never_on_a_wobble() {
        let zooming_in = changes_over((0..=800).map(|step| 1000.0 * 0.99f64.powi(step)));
        let zooming_out = changes_over((0..=800).map(|step| 1000.0 * 1.01f64.powi(step)));
        let wobbling =
            changes_over((0..=800).map(|step| 1000.0 * (1.0 + 0.3 * f64::from(step).sin())));
        let halvings = 800.0 * -(0.99f64.log2());
        let doublings = 800.0 * 1.01f64.log2();

        assert!(
            (f64::from(zooming_in) - halvings).abs() <= 1.0,
            "{zooming_in} in {halvings}"
        );
        assert!(
            (f64::from(zooming_out) - doublings / 2.0).abs() <= 1.0,
            "{zooming_out} in {doublings}"
        );
        assert!(wobbling <= 1, "{wobbling}");
    }
}
