use std::f64::consts::FRAC_PI_2;

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::{
    BSpline, BSplineSurface, Circle, Cone, Curve, Cylinder, Extrusion, GeometryError, Interval,
    LINEAR_RESOLUTION, Line, PlaneSurface, Revolution, Sphere, Surface, Torus, check_interrupt,
};

use crate::read::topology::short;

const FIT_DEGREE: usize = 3;
const FIRST_PIECES: usize = 4;
const MAX_CURVE_SAMPLES: usize = 16_385;
const MAX_SURFACE_SAMPLES: usize = 513;
const TIGHT_FIT: f64 = 0.25 * LINEAR_RESOLUTION;
const POLE_NUDGE: f64 = 1e-7;
const RADIAL_SLACK: f64 = 1e-9;
const SAMPLES_PER_CHARGE: usize = 16;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub(crate) enum OffsetRefusal {
    #[error("is offset by more than the radius of its {0}, which leaves no surface")]
    PastCentre(&'static str),
    #[error("is offset so far that the tube of its torus reaches the axis")]
    TubeReachesAxis,
    #[error("is an offset of a {0} with no end, which caditor cannot import yet")]
    Endless(&'static str),
    #[error("is an offset of a surface of its kind, which caditor cannot import yet")]
    Unsupported,
    #[error("is offset farther than its {0} curves, so the offset folds over itself")]
    Folds(&'static str),
    #[error("is offset from a point where its {0} has no direction across it")]
    NoNormal(&'static str),
    #[error(
        "is an offset of a {what} that caditor could not match within {} mm (its closest match is \
         {} mm off)",
        short(*.within),
        short(*.reached)
    )]
    NotFitted {
        what: &'static str,
        within: f64,
        reached: f64,
    },
    #[error("is part of a model too intricate to import in one go")]
    TooIntricate,
    #[error("was not read, because the import was cancelled")]
    Cancelled,
    #[error("is not a usable surface ({0})")]
    Kernel(#[from] GeometryError),
}

pub(crate) struct Fit<'c> {
    pub tolerance: f64,
    pub charge: &'c dyn Fn(usize) -> Result<(), OffsetRefusal>,
}

impl Fit<'_> {
    fn round(&self, samples: usize) -> Result<(), OffsetRefusal> {
        check_interrupt().map_err(|_| OffsetRefusal::Cancelled)?;
        (self.charge)(samples / SAMPLES_PER_CHARGE + 1)
    }
}

pub(crate) fn offset(
    basis: &Surface,
    distance: f64,
    fit: &Fit<'_>,
) -> Result<Surface, OffsetRefusal> {
    let grown = |radius: f64, what: &'static str| {
        let offset = radius + distance;
        if offset > 0.0 {
            Ok(offset)
        } else {
            Err(OffsetRefusal::PastCentre(what))
        }
    };
    Ok(match basis {
        Surface::Plane(plane) => {
            let frame = plane.frame();
            let moved = Plane::from_frame(
                frame.origin() + frame.normal() * distance,
                frame.normal(),
                frame.x_axis(),
            )
            .ok_or(GeometryError::NonFinite)?;
            PlaneSurface::new(moved)?.into()
        }
        Surface::Cylinder(cylinder) => {
            Cylinder::new(*cylinder.frame(), grown(cylinder.radius(), "cylinder")?)?.into()
        }
        Surface::Sphere(sphere) => {
            Sphere::new(*sphere.frame(), grown(sphere.radius(), "sphere")?)?.into()
        }
        Surface::Torus(torus) => {
            let minor = grown(torus.minor_radius(), "torus's tube")?;
            if minor >= torus.major_radius() {
                return Err(OffsetRefusal::TubeReachesAxis);
            }
            Torus::new(*torus.frame(), torus.major_radius(), minor)?.into()
        }
        Surface::Cone(cone) => {
            let frame = cone.frame();
            let (sin, cos) = cone.half_angle().sin_cos();
            let radius = cone.radius() + distance * cos;
            let (slide, radius) = if radius < 0.0 {
                (radius / sin, 0.0)
            } else {
                (0.0, radius)
            };
            let origin = frame.origin() - frame.normal() * (distance * sin + slide * cos);
            let moved = Plane::from_frame(origin, frame.normal(), frame.x_axis())
                .ok_or(GeometryError::NonFinite)?;
            Cone::new(moved, radius, cone.half_angle())?.into()
        }
        Surface::Extrusion(extrusion) => {
            let what = "surface of extrusion";
            let (profile, samples) = offset_profile(
                extrusion.profile(),
                |t| basis.normal(t, 0.0),
                distance,
                fit,
                what,
            )?;
            let moved = Surface::from(Extrusion::new(profile, extrusion.direction())?);
            unfolded(
                basis,
                &moved,
                samples.iter().map(|t| Point2::new(*t, 0.0)),
                what,
            )?;
            moved
        }
        Surface::Revolution(revolution) => {
            let what = "surface of revolution";
            let (profile, samples) = offset_profile(
                revolution.profile(),
                |t| basis.normal(0.0, t),
                distance,
                fit,
                what,
            )?;
            let moved = Surface::from(Revolution::new(
                profile,
                revolution.axis_origin(),
                revolution.axis_direction(),
            )?);
            unfolded(
                basis,
                &moved,
                samples.iter().map(|t| Point2::new(0.0, *t)),
                what,
            )?;
            moved
        }
        Surface::BSpline(spline) => fitted_surface(basis, spline, distance, fit)?,
        _ => return Err(OffsetRefusal::Unsupported),
    })
}

fn unfolded(
    basis: &Surface,
    moved: &Surface,
    samples: impl Iterator<Item = Point2>,
    what: &'static str,
) -> Result<(), OffsetRefusal> {
    for uv in samples {
        if let (Some(before), Some(after)) = (basis.normal(uv.x, uv.y), moved.normal(uv.x, uv.y))
            && before.dot(after) <= 0.0
        {
            return Err(OffsetRefusal::Folds(what));
        }
    }
    Ok(())
}

fn offset_profile(
    profile: &Curve,
    normal: impl Fn(f64) -> Option<Vector3>,
    distance: f64,
    fit: &Fit<'_>,
    what: &'static str,
) -> Result<(Curve, Vec<f64>), OffsetRefusal> {
    match profile {
        Curve::Line(line) => {
            let across = normal(0.0).ok_or(OffsetRefusal::NoNormal(what))?;
            let moved = Line::new(line.origin() + across * distance, line.direction())?;
            return Ok((moved.into(), vec![0.0]));
        }
        Curve::Circle(circle) => {
            if let Some(sign) = radial_sign(profile, circle, &normal) {
                let radius = circle.radius() + sign * distance;
                if radius <= 0.0 {
                    return Err(OffsetRefusal::PastCentre("profile's circle"));
                }
                let moved = Circle::new(*circle.frame(), radius)?;
                return Ok((moved.into(), circle_samples().to_vec()));
            }
        }
        _ => {}
    }
    let range = profile
        .domain()
        .bounded()
        .or_else(|| {
            profile
                .period()
                .and_then(|period| Interval::new(0.0, period))
        })
        .ok_or(OffsetRefusal::Endless(what))?;
    let breaks = match profile {
        Curve::BSpline(spline) => spline.breakpoints(),
        _ => vec![range.start(), range.end()],
    };
    let exact = |t: f64| -> Result<Point3, OffsetRefusal> {
        let across = normal(t)
            .or_else(|| normal(nudged(t, range)))
            .ok_or(OffsetRefusal::NoNormal(what))?;
        Ok(profile.point(t) + across * distance)
    };
    let (fitted, samples) = fitted_curve(&breaks, exact, fit, what)?;
    Ok((Curve::BSpline(fitted), samples))
}

fn circle_samples() -> [f64; 4] {
    [0.0, FRAC_PI_2, 2.0 * FRAC_PI_2, 3.0 * FRAC_PI_2]
}

fn radial_sign(
    profile: &Curve,
    circle: &Circle,
    normal: &impl Fn(f64) -> Option<Vector3>,
) -> Option<f64> {
    let mut sign = None;
    for t in circle_samples() {
        let radial = (profile.point(t) - circle.center()).try_normalize()?;
        let along = normal(t)?.dot(radial);
        if along.abs() < 1.0 - RADIAL_SLACK {
            return None;
        }
        let this = along.signum();
        if sign.is_some_and(|sign| sign != this) {
            return None;
        }
        sign = Some(this);
    }
    sign
}

fn nudged(t: f64, range: Interval) -> f64 {
    let step = POLE_NUDGE * range.length();
    if t - range.start() < range.end() - t {
        t + step
    } else {
        t - step
    }
}

fn spans(breaks: &[f64], most: usize) -> Vec<f64> {
    let count = breaks.len().saturating_sub(1);
    if count <= most {
        return breaks.to_vec();
    }
    let mut kept: Vec<f64> = (0..=most)
        .filter_map(|index| breaks.get(index * count / most).copied())
        .collect();
    kept.dedup();
    kept
}

fn sampled(breaks: &[f64], pieces: usize) -> Vec<f64> {
    let mut parameters = Vec::with_capacity(breaks.len() * pieces);
    for pair in breaks.windows(2) {
        if let [start, end] = pair
            && let Some(span) = Interval::new(*start, *end)
        {
            if !parameters.is_empty() {
                parameters.pop();
            }
            parameters.extend(span.split(pieces));
        }
    }
    parameters
}

fn middles(parameters: &[f64]) -> Vec<f64> {
    parameters
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] => Some(0.5 * (a + b)),
            _ => None,
        })
        .collect()
}

fn joined(
    parameters: &[f64],
    points: &[Point3],
    pieces: usize,
) -> Result<(Vec<f64>, Vec<Point3>), GeometryError> {
    let mut knots: Vec<f64> = Vec::new();
    let mut control: Vec<Point3> = Vec::new();
    let mut start = 0;
    while start + pieces < parameters.len() {
        let end = start + pieces;
        let piece = BSpline::interpolating_at(
            FIT_DEGREE,
            parameters.get(start..=end).ok_or(GeometryError::Knots)?,
            points.get(start..=end).ok_or(GeometryError::Knots)?,
        )?;
        if knots.is_empty() {
            knots.extend_from_slice(piece.knots());
            control.extend_from_slice(piece.control_points());
        } else {
            knots.pop();
            knots.extend(piece.knots().iter().skip(FIT_DEGREE + 1));
            control.extend(piece.control_points().iter().skip(1));
        }
        start = end;
    }
    Ok((knots, control))
}

fn fitted_curve(
    breaks: &[f64],
    exact: impl Fn(f64) -> Result<Point3, OffsetRefusal>,
    fit: &Fit<'_>,
    what: &'static str,
) -> Result<(BSpline<Point3>, Vec<f64>), OffsetRefusal> {
    let breaks = spans(breaks, (MAX_CURVE_SAMPLES - 1) / FIRST_PIECES);
    let mut pieces = FIRST_PIECES;
    loop {
        let parameters = sampled(&breaks, pieces);
        fit.round(parameters.len())?;
        let points = parameters
            .iter()
            .map(|t| exact(*t))
            .collect::<Result<Vec<Point3>, OffsetRefusal>>()?;
        let (knots, control) = joined(&parameters, &points, pieces)?;
        let curve = BSpline::new(FIT_DEGREE, knots, control)?;
        let mut reached: f64 = 0.0;
        for t in middles(&parameters) {
            reached = reached.max(curve.point(t).distance(exact(t)?));
        }
        let finer = 2 * parameters.len() - 1;
        if reached <= TIGHT_FIT || finer > MAX_CURVE_SAMPLES {
            if reached > fit.tolerance {
                return Err(OffsetRefusal::NotFitted {
                    what,
                    within: fit.tolerance,
                    reached,
                });
            }
            let mut samples = parameters;
            samples.extend(middles(&samples));
            return Ok((curve, samples));
        }
        pieces *= 2;
    }
}

struct Grid {
    u: Vec<f64>,
    v: Vec<f64>,
    u_pieces: usize,
    v_pieces: usize,
}

fn fitted_surface(
    basis: &Surface,
    spline: &BSplineSurface,
    distance: f64,
    fit: &Fit<'_>,
) -> Result<Surface, OffsetRefusal> {
    let what = "spline surface";
    let (u_domain, v_domain) = (spline.u_domain(), spline.v_domain());
    let most = (MAX_SURFACE_SAMPLES - 1) / FIRST_PIECES;
    let breaks_of = |knots: &[f64], domain: Interval| {
        let mut breaks: Vec<f64> = knots
            .iter()
            .copied()
            .filter(|knot| domain.contains(*knot))
            .collect();
        breaks.dedup();
        spans(&breaks, most)
    };
    let u_breaks = breaks_of(spline.u_knots(), u_domain);
    let v_breaks = breaks_of(spline.v_knots(), v_domain);
    let exact = |u: f64, v: f64| -> Result<(Point3, Vector3), OffsetRefusal> {
        let across = basis
            .normal(u, v)
            .or_else(|| basis.normal(nudged(u, u_domain), v))
            .or_else(|| basis.normal(u, nudged(v, v_domain)))
            .or_else(|| basis.normal(nudged(u, u_domain), nudged(v, v_domain)))
            .ok_or(OffsetRefusal::NoNormal(what))?;
        Ok((
            basis.point_at(Point2::new(u, v)) + across * distance,
            across,
        ))
    };
    let (mut u_pieces, mut v_pieces) = (FIRST_PIECES, FIRST_PIECES);
    loop {
        let grid = Grid {
            u: sampled(&u_breaks, u_pieces),
            v: sampled(&v_breaks, v_pieces),
            u_pieces,
            v_pieces,
        };
        fit.round(grid.u.len() * grid.v.len())?;
        let fitted = Surface::BSpline(surface_through(basis, &grid, &exact)?);
        let mut reached = [0.0f64; 2];
        let (u_middles, v_middles) = (middles(&grid.u), middles(&grid.v));
        let checks = [
            (&u_middles, &grid.v, [true, false]),
            (&grid.u, &v_middles, [false, true]),
            (&u_middles, &v_middles, [true, true]),
        ];
        for (us, vs, refines) in checks {
            for u in us {
                for v in vs {
                    let (point, normal) = exact(*u, *v)?;
                    let gap = fitted.point_at(Point2::new(*u, *v)).distance(point);
                    if fitted
                        .normal(*u, *v)
                        .is_some_and(|fitted| fitted.dot(normal) <= 0.0)
                    {
                        return Err(OffsetRefusal::Folds(what));
                    }
                    for (slot, refined) in reached.iter_mut().zip(refines) {
                        if refined {
                            *slot = slot.max(gap);
                        }
                    }
                }
            }
        }
        let grows = |count: usize| 2 * count - 1 <= MAX_SURFACE_SAMPLES;
        let refine_u = reached[0] > TIGHT_FIT && grows(grid.u.len());
        let refine_v = reached[1] > TIGHT_FIT && grows(grid.v.len());
        if !refine_u && !refine_v {
            let reached = reached[0].max(reached[1]);
            if reached > fit.tolerance {
                return Err(OffsetRefusal::NotFitted {
                    what,
                    within: fit.tolerance,
                    reached,
                });
            }
            return Ok(fitted);
        }
        if refine_u {
            u_pieces *= 2;
        }
        if refine_v {
            v_pieces *= 2;
        }
    }
}

fn surface_through(
    basis: &Surface,
    grid: &Grid,
    exact: &impl Fn(f64, f64) -> Result<(Point3, Vector3), OffsetRefusal>,
) -> Result<BSplineSurface, OffsetRefusal> {
    let mut rows: Vec<Vec<Point3>> = Vec::with_capacity(grid.v.len());
    let mut on_basis: Vec<Vec<Point3>> = Vec::with_capacity(grid.v.len());
    for v in &grid.v {
        let mut row = Vec::with_capacity(grid.u.len());
        let mut basis_row = Vec::with_capacity(grid.u.len());
        for u in &grid.u {
            row.push(exact(*u, *v)?.0);
            basis_row.push(basis.point_at(Point2::new(*u, *v)));
        }
        rows.push(row);
        on_basis.push(basis_row);
    }
    gather_poles(&mut rows, &on_basis);
    let mut u_knots = Vec::new();
    let mut control_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let (knots, control) = joined(&grid.u, row, grid.u_pieces)?;
        u_knots = knots;
        control_rows.push(control);
    }
    let columns = control_rows.first().map_or(0, Vec::len);
    let mut v_knots = Vec::new();
    let mut control_columns = Vec::with_capacity(columns);
    for column in 0..columns {
        let values: Vec<Point3> = control_rows
            .iter()
            .filter_map(|row| row.get(column).copied())
            .collect();
        let (knots, control) = joined(&grid.v, &values, grid.v_pieces)?;
        v_knots = knots;
        control_columns.push(control);
    }
    let count = control_columns.first().map_or(0, Vec::len);
    let mut points = Vec::with_capacity(count * columns);
    for row in 0..count {
        for column in &control_columns {
            points.push(*column.get(row).ok_or(GeometryError::Knots)?);
        }
    }
    Ok(BSplineSurface::new(
        FIT_DEGREE, FIT_DEGREE, u_knots, v_knots, columns, points, None,
    )?)
}

fn gather_poles(rows: &mut [Vec<Point3>], on_basis: &[Vec<Point3>]) {
    let collapsed = |points: &[Point3]| {
        points.first().is_some_and(|first| {
            points
                .iter()
                .all(|point| point.distance(*first) <= LINEAR_RESOLUTION)
        })
    };
    let mean = |points: &[Point3]| {
        let count = points.len().max(1) as f64;
        points.iter().fold(Point3::ZERO, |sum, point| sum + *point) / count
    };
    for (row, basis_row) in rows.iter_mut().zip(on_basis) {
        if collapsed(basis_row) {
            let pole = mean(row);
            row.iter_mut().for_each(|point| *point = pole);
        }
    }
    let columns = on_basis.first().map_or(0, Vec::len);
    for column in 0..columns {
        let basis_column: Vec<Point3> = on_basis
            .iter()
            .filter_map(|row| row.get(column).copied())
            .collect();
        if !collapsed(&basis_column) {
            continue;
        }
        let column_points: Vec<Point3> = rows
            .iter()
            .filter_map(|row| row.get(column).copied())
            .collect();
        let pole = mean(&column_points);
        for row in rows.iter_mut() {
            if let Some(point) = row.get_mut(column) {
                *point = pole;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use caditor_kernel::Interval;

    use super::*;

    fn frame() -> Plane {
        Plane::from_frame(
            Point3::new(1.0, -2.0, 3.0),
            Vector3::new(0.2, 0.3, 1.0),
            Vector3::X,
        )
        .unwrap()
    }

    fn free(_: usize) -> Result<(), OffsetRefusal> {
        Ok(())
    }

    fn within(tolerance: f64) -> Fit<'static> {
        Fit {
            tolerance,
            charge: &free,
        }
    }

    fn assert_offset(name: &str, basis: Surface, distance: f64, u: Interval, v: Interval) {
        let offset = offset(&basis, distance, &within(1e-6)).unwrap();
        for row in 0..=12 {
            for column in 0..=12 {
                let (u, v) = (u.at(column as f64 / 12.0), v.at(row as f64 / 12.0));
                let Some(normal) = basis.normal(u, v) else {
                    continue;
                };
                let moved = basis.point_at(Point2::new(u, v)) + normal * distance;
                let foot = offset.project(moved, None);

                assert!(
                    offset.point_at(foot).distance(moved) < 1e-6,
                    "{name}: {moved} is off its offset by {}",
                    offset.point_at(foot).distance(moved)
                );
                assert!(
                    offset.normal(foot.x, foot.y).unwrap().dot(normal) > 1.0 - 1e-6,
                    "{name}: the normal turned at {moved}"
                );
            }
        }
    }

    #[test]
    fn offsets_of_elementary_surfaces_are_exact_and_keep_their_normals() {
        let span = |start: f64, end: f64| Interval::new(start, end).unwrap();
        let turn = span(0.0, 6.0);
        let cone =
            |radius: f64, angle: f64| Surface::Cone(Cone::new(frame(), radius, angle).unwrap());

        assert_offset(
            "plane",
            Surface::Plane(PlaneSurface::new(frame()).unwrap()),
            -2.5,
            span(-6.0, 6.0),
            span(-5.0, 5.0),
        );
        for distance in [2.0, -3.0] {
            assert_offset(
                "cylinder",
                Surface::Cylinder(Cylinder::new(frame(), 5.0).unwrap()),
                distance,
                turn,
                span(-5.0, 5.0),
            );
            assert_offset(
                "sphere",
                Surface::Sphere(Sphere::new(frame(), 5.0).unwrap()),
                distance,
                turn,
                span(-1.4, 1.4),
            );
            assert_offset(
                "torus",
                Surface::Torus(Torus::new(frame(), 10.0, 4.0).unwrap()),
                distance,
                turn,
                span(0.0, 6.0),
            );
        }
        assert_offset("cone", cone(4.0, 0.5), 1.5, turn, span(-2.0, 6.0));
        assert_offset(
            "cone past its apex",
            cone(4.0, 0.5),
            -10.0,
            turn,
            span(11.0, 20.0),
        );
        assert_offset(
            "opening downwards",
            cone(4.0, -0.4),
            -6.0,
            turn,
            span(-20.0, -12.0),
        );
    }

    fn wave() -> BSpline<Point3> {
        BSpline::new(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(10.0, -6.0, 0.0),
                Point3::new(20.0, 8.0, 0.0),
                Point3::new(30.0, -4.0, 0.0),
                Point3::new(40.0, 0.0, 0.0),
            ],
        )
        .unwrap()
    }

    #[test]
    fn offsets_of_extrusions_revolutions_and_splines_are_fitted_within_the_tolerance() {
        let unit = Interval::UNIT;
        let span = |start: f64, end: f64| Interval::new(start, end).unwrap();
        let extruded =
            Surface::Extrusion(Extrusion::new(Curve::BSpline(wave()), Vector3::Z).unwrap());
        let ellipse = caditor_kernel::Ellipse::new(Plane::XY, 8.0, 5.0).unwrap();
        let oval = Surface::Extrusion(
            Extrusion::new(Curve::Ellipse(ellipse), Vector3::new(0.0, 0.3, 1.0)).unwrap(),
        );
        let profile = BSpline::new(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            vec![
                Point3::new(4.0, 0.0, 0.0),
                Point3::new(9.0, 0.0, 3.0),
                Point3::new(3.0, 0.0, 7.0),
                Point3::new(6.0, 0.0, 10.0),
            ],
        )
        .unwrap();
        let vase = Surface::Revolution(
            Revolution::new(Curve::BSpline(profile), Point3::ZERO, Vector3::Z).unwrap(),
        );
        let mut net = Vec::new();
        for row in 0..4 {
            for column in 0..5 {
                let (x, y) = (column as f64 * 5.0, row as f64 * 6.0);
                net.push(Point3::new(x, y, 3.0 * (x * 0.3).sin() * (y * 0.2).cos()));
            }
        }
        let bump = Surface::BSpline(
            BSplineSurface::new(
                3,
                3,
                vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                5,
                net,
                None,
            )
            .unwrap(),
        );

        assert_offset("extrusion", extruded.clone(), 1.5, unit, span(-5.0, 5.0));
        assert_offset("extrusion inwards", extruded, -1.0, unit, span(0.0, 3.0));
        assert_offset("oval", oval, 1.0, span(0.0, TAU), span(-2.0, 2.0));
        assert_offset("vase", vase, 0.8, span(0.0, 6.0), unit);
        assert_offset("bump", bump, 1.2, unit, unit);
    }

    #[test]
    fn offsets_that_leave_no_surface_or_fold_over_are_refused_in_words() {
        let cylinder = Surface::Cylinder(Cylinder::new(frame(), 2.0).unwrap());
        let torus = Surface::Torus(Torus::new(frame(), 10.0, 3.0).unwrap());
        let tight = Surface::Extrusion(Extrusion::new(Curve::BSpline(wave()), Vector3::Z).unwrap());

        let refusal = |basis: &Surface, distance: f64| {
            offset(basis, distance, &within(1e-6))
                .unwrap_err()
                .to_string()
        };

        assert_eq!(
            refusal(&cylinder, -2.0),
            "is offset by more than the radius of its cylinder, which leaves no surface"
        );
        assert_eq!(
            refusal(&torus, 8.0),
            "is offset so far that the tube of its torus reaches the axis"
        );
        assert_eq!(
            refusal(&tight, 12.0),
            "is offset farther than its surface of extrusion curves, so the offset folds over \
             itself"
        );
    }

    #[test]
    fn a_fit_that_cannot_reach_the_tolerance_says_how_close_it_came() {
        let bumpy = Surface::Extrusion(Extrusion::new(Curve::BSpline(wave()), Vector3::Z).unwrap());

        let refusal = offset(&bumpy, 1.0, &within(1e-9)).unwrap_err();

        assert!(
            matches!(refusal, OffsetRefusal::NotFitted { reached, .. } if reached > 0.0),
            "{refusal:?}"
        );
        assert!(
            refusal
                .to_string()
                .starts_with("is an offset of a surface of extrusion that caditor could not match"),
            "{refusal}"
        );
    }
}
