use caditor_geometry::Point3;
use caditor_kernel::{BSpline, Curve, Interval, LINEAR_RESOLUTION, Line, Surface};

const CLEAN: f64 = 0.25 * LINEAR_RESOLUTION;
const FIRST_SAMPLES: usize = 8;
const MOST_SAMPLES: usize = 1024;
const FIT_SHARE: f64 = 0.1;

pub(crate) fn exact_kind(surface: &Surface) -> bool {
    !matches!(
        surface,
        Surface::BSpline(_) | Surface::Extrusion(_) | Surface::Revolution(_)
    )
}

pub(crate) fn farthest(surface: &Surface, samples: &[Point3]) -> f64 {
    let mut hint = None;
    samples
        .iter()
        .map(|point| {
            let hinted = hint.map(|hint| surface.project(*point, Some(hint)));
            let foot = match hinted {
                Some(uv) if surface.point_at(uv).distance(*point) <= CLEAN => uv,
                _ => surface.project(*point, None),
            };
            hint = Some(foot);
            surface.point_at(foot).distance(*point)
        })
        .fold(0.0, f64::max)
}

pub(crate) fn met_at_ends(
    curve: &Curve,
    interval: Interval,
    [from, to]: [Point3; 2],
    allowance: f64,
) -> Option<((Curve, Interval), f64)> {
    let shifts = [
        from - curve.point(interval.start()),
        to - curve.point(interval.end()),
    ];
    let gap = shifts[0].length().max(shifts[1].length());
    if gap <= CLEAN || gap > allowance {
        return None;
    }
    if let Curve::Line(_) = curve
        && let Ok(line) = Line::through(from, to)
        && let Some(length) = Interval::new(0.0, from.distance(to))
    {
        return Some(((Curve::Line(line), length), gap));
    }
    let blended = |along: f64| {
        curve.point(interval.at(along)) + shifts[0] * (1.0 - along) + shifts[1] * along
    };
    let fit = (FIT_SHARE * allowance).max(LINEAR_RESOLUTION);
    let mut count = FIRST_SAMPLES;
    while count <= MOST_SAMPLES {
        let points: Vec<Point3> = (0..=count)
            .map(|index| blended(index as f64 / count as f64))
            .collect();
        if let Ok(spline) = BSpline::interpolating(3, &points) {
            let domain = spline.domain();
            let rebuilt = Curve::BSpline(spline);
            let close = (0..count).all(|index| {
                let along = (index as f64 + 0.5) / count as f64;
                let target = blended(along);
                let near = rebuilt.closest_parameter(target, domain);
                rebuilt.point(near).distance(target) <= fit
            });
            if close {
                return Some(((rebuilt, domain), gap));
            }
        }
        count *= 2;
    }
    None
}
