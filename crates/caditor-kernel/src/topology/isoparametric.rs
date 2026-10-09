use caditor_geometry::{Aabb2, Point2};

use crate::{
    interrupt::{self, Interrupted},
    interval::Interval,
    topology::{FaceContainment, FaceId, Solid, SolidClassifier},
};

pub const MOST_ISOPARAMETRIC_LINES: usize = 64;
const SAMPLES_PER_LINE: usize = 96;
const END_BISECTIONS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Along {
    U,
    V,
}

impl Along {
    fn uv(self, fixed: f64, parameter: f64) -> Point2 {
        match self {
            Self::U => Point2::new(parameter, fixed),
            Self::V => Point2::new(fixed, parameter),
        }
    }

    fn spans(self, uv_box: &Aabb2) -> Option<(Interval, Interval)> {
        let (min, max) = (uv_box.min(), uv_box.max());
        let u = Interval::new(min.x, max.x)?;
        let v = Interval::new(min.y, max.y)?;
        Some(match self {
            Self::U => (v, u),
            Self::V => (u, v),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsoparametricRun {
    pub along: Along,
    pub fixed: f64,
    pub span: Interval,
}

impl IsoparametricRun {
    pub fn uv(&self, parameter: f64) -> Point2 {
        self.along.uv(self.fixed, parameter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IsoparametricError {
    #[error("the face is not part of the solid or has no boundary")]
    MissingFace,
    #[error("finding the parameter lines was cancelled")]
    Cancelled,
}

impl From<Interrupted> for IsoparametricError {
    fn from(_: Interrupted) -> Self {
        Self::Cancelled
    }
}

struct Lines<'a> {
    classifier: SolidClassifier<'a>,
    face: FaceId,
}

impl Lines<'_> {
    fn inside(&self, along: Along, fixed: f64, parameter: f64) -> bool {
        matches!(
            self.classifier
                .point_in_face(self.face, along.uv(fixed, parameter)),
            Some(FaceContainment::Inside | FaceContainment::OnBoundary)
        )
    }

    fn boundary(&self, along: Along, fixed: f64, inside: f64, outside: f64) -> f64 {
        let (mut inside, mut outside) = (inside, outside);
        for _ in 0..END_BISECTIONS {
            let middle = 0.5 * (inside + outside);
            if self.inside(along, fixed, middle) {
                inside = middle;
            } else {
                outside = middle;
            }
        }
        inside
    }

    fn runs_of_line(&self, along: Along, fixed: f64, span: Interval) -> Vec<IsoparametricRun> {
        let parameter_at = |sample: usize| span.at(sample as f64 / SAMPLES_PER_LINE as f64);
        let inside: Vec<bool> = (0..=SAMPLES_PER_LINE)
            .map(|sample| self.inside(along, fixed, parameter_at(sample)))
            .collect();
        let mut runs = Vec::new();
        let mut start = None;
        for (sample, is_inside) in inside.iter().copied().chain([false]).enumerate() {
            match (is_inside, start) {
                (true, None) => start = Some(sample),
                (false, Some(first)) => {
                    let last = sample - 1;
                    let from = match first {
                        0 => span.start(),
                        _ => self.boundary(
                            along,
                            fixed,
                            parameter_at(first),
                            parameter_at(first - 1),
                        ),
                    };
                    let to = if last >= SAMPLES_PER_LINE {
                        span.end()
                    } else {
                        self.boundary(along, fixed, parameter_at(last), parameter_at(last + 1))
                    };
                    if let Some(run_span) = Interval::new(from, to).filter(|run| run.length() > 0.0)
                    {
                        runs.push(IsoparametricRun {
                            along,
                            fixed,
                            span: run_span,
                        });
                    }
                    start = None;
                }
                _ => {}
            }
        }
        runs
    }
}

impl Solid {
    pub fn isoparametric_runs(
        &self,
        face: FaceId,
        along: Along,
        lines: usize,
    ) -> Result<Vec<IsoparametricRun>, IsoparametricError> {
        let classifier =
            SolidClassifier::of_face(self, face).ok_or(IsoparametricError::MissingFace)?;
        let uv_box = classifier
            .face_uv_box(face)
            .ok_or(IsoparametricError::MissingFace)?;
        let (across, span) = along
            .spans(&uv_box)
            .ok_or(IsoparametricError::MissingFace)?;
        let lines_found = Lines { classifier, face };
        let count = lines.clamp(1, MOST_ISOPARAMETRIC_LINES);
        let mut runs = Vec::new();
        for line in 0..count {
            interrupt::check()?;
            let fixed = across.at((line + 1) as f64 / (count + 1) as f64);
            runs.extend(lines_found.runs_of_line(along, fixed, span));
        }
        Ok(runs)
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Point3;

    use super::*;
    use crate::{
        fixtures::{cylinder, holed_block},
        surface::Surface,
        test_support::cancelled_after,
    };

    fn face_where(solid: &Solid, test: impl Fn(&Surface) -> bool) -> FaceId {
        solid
            .faces()
            .find(|(_, face)| test(face.surface()))
            .map(|(id, _)| id)
            .unwrap()
    }

    #[test]
    fn lines_across_a_face_with_a_hole_stop_at_the_hole_and_the_outline() {
        let solid = holed_block(10.0, 4.0, 2.0);
        let bottom = solid
            .faces()
            .find(|(_, face)| {
                face.inner_loops().len() == 1
                    && face.surface().point(0.0, 0.0).z.abs() < 1e-9
                    && matches!(face.surface(), Surface::Plane(_))
            })
            .map(|(id, _)| id)
            .unwrap();
        let surface = solid.face(bottom).unwrap().surface().clone();
        let centre = Point3::new(5.0, 5.0, 0.0);

        for along in [Along::U, Along::V] {
            let runs = solid.isoparametric_runs(bottom, along, 3).unwrap();

            assert_eq!(runs.len(), 4, "{runs:?}");
            for run in &runs {
                assert_eq!(run.along, along);
                for end in [run.span.start(), run.span.end()] {
                    let point = surface.point_at(run.uv(end));
                    let on_hole = (point.distance(centre) - 2.0).abs() < 1e-6;
                    let on_outline = [point.x, point.y]
                        .iter()
                        .any(|value| value.abs() < 1e-6 || (value - 10.0).abs() < 1e-6);
                    assert!(on_hole || on_outline, "{point:?}");
                }
            }
        }
    }

    #[test]
    fn a_cylinder_side_has_whole_circles_one_way_and_whole_rulings_the_other() {
        let solid = cylinder(3.0, 8.0);
        let side = face_where(&solid, |surface| matches!(surface, Surface::Cylinder(_)));
        let surface = solid.face(side).unwrap().surface().clone();

        let circles = solid.isoparametric_runs(side, Along::U, 4).unwrap();
        let rulings = solid.isoparametric_runs(side, Along::V, 5).unwrap();

        assert_eq!(circles.len(), 4);
        assert!(
            circles
                .iter()
                .all(|run| (run.span.length() - std::f64::consts::TAU).abs() < 1e-9)
        );
        assert_eq!(rulings.len(), 5);
        for run in &rulings {
            let ends = [run.span.start(), run.span.end()].map(|end| surface.point_at(run.uv(end)));
            assert!((ends[0].distance(ends[1]) - 8.0).abs() < 1e-9);
        }
        let heights: Vec<f64> = circles
            .iter()
            .map(|run| surface.point_at(run.uv(run.span.start())).z)
            .collect();
        assert!(
            heights
                .iter()
                .zip([1.6, 3.2, 4.8, 6.4])
                .all(|(height, expected)| (height - expected).abs() < 1e-9),
            "{heights:?}"
        );
    }

    #[test]
    fn a_missing_face_is_an_error_and_the_search_can_be_cancelled() {
        let solid = cylinder(3.0, 8.0);
        let side = face_where(&solid, |surface| matches!(surface, Surface::Cylinder(_)));

        let (cancelled, _) = cancelled_after(0, || solid.isoparametric_runs(side, Along::U, 4));

        assert_eq!(cancelled, Err(IsoparametricError::Cancelled));
        assert_eq!(
            cylinder(1.0, 1.0)
                .isoparametric_runs(FaceId::from_index(99).unwrap(), Along::U, 4)
                .map(|runs| runs.len()),
            Err(IsoparametricError::MissingFace)
        );
    }
}
