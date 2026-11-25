use std::{borrow::Cow, collections::BTreeMap, f64::consts::FRAC_PI_2};

use caditor_geometry::{Aabb, Point2, Point3, Vector2, Vector3};

use super::{
    MeasureError,
    quadrature::{self, Budget, COMPONENTS, Quadrature, Tolerance, Values, scaled},
};
use crate::{
    curve::Curve,
    interrupt,
    interval::Interval,
    surface::{BSplineSurface, Surface, SurfaceDerivatives},
    tessellation::{MassProperties, Mesh, Moments, TessellationError},
    tolerance::{LINEAR_RESOLUTION, SamplingTolerance},
    topology::{Coedge, FaceId, Solid},
};

const BOUNDARY_TOLERANCE: f64 = 1e-12;
const BOUNDARY_NOISE: f64 = 1e-10;
const STRIP_TOLERANCE: f64 = 1e-14;
const FACE_EVALUATIONS: usize = 1 << 23;
const QUARTER_TURN: f64 = FRAC_PI_2;
const LEVEL_RATE: f64 = 1e-15;
const MAX_CROSSINGS: usize = 4096;
const MAX_BISECTIONS: usize = 80;
const BISECTION_EPSILON: f64 = 1e-15;
const KINK_SLACK: f64 = 1e-9;
const LEVEL_SLACK: f64 = 1e-10;
const GAP_LIMIT: f64 = 1e-6;

#[derive(Clone, Debug, PartialEq)]
pub struct SolidMass {
    pub properties: MassProperties,
    pub face_areas: BTreeMap<FaceId, f64>,
    pub meshed_faces: Vec<FaceId>,
}

pub fn mass_properties(solid: &Solid, mesh: &Mesh) -> Result<SolidMass, MeasureError> {
    mass_properties_meshing(solid, || Ok(Cow::Borrowed(mesh)))
}

pub fn mass_properties_tessellating(
    solid: &Solid,
    tolerance: &SamplingTolerance,
) -> Result<SolidMass, MeasureError> {
    mass_properties_meshing(solid, || {
        solid
            .tessellate(tolerance)
            .map(Cow::Owned)
            .map_err(|error| match error {
                TessellationError::Cancelled(interrupted) => MeasureError::Cancelled(interrupted),
                other => MeasureError::Meshing(other),
            })
    })
}

fn mass_properties_meshing<'a>(
    solid: &Solid,
    mesh: impl FnOnce() -> Result<Cow<'a, Mesh>, MeasureError>,
) -> Result<SolidMass, MeasureError> {
    let frame = Frame::of(solid);
    let mut integrated = Vec::new();

    for (face, _) in solid.faces() {
        interrupt::check()?;
        integrated.push((face, face_moments(solid, face, &frame)?));
    }

    let mesh = if integrated.iter().any(|(_, moments)| moments.is_none()) {
        Some(mesh()?)
    } else {
        None
    };
    let mut moments = Moments::default();
    let mut face_areas = BTreeMap::new();
    let mut meshed_faces = Vec::new();

    for (face, found) in integrated {
        match (found, &mesh) {
            (Some(found), _) => {
                moments.add(&found);
                face_areas.insert(face, found.area);
            }
            (None, Some(mesh)) => {
                let triangles: Vec<[Point3; 3]> =
                    mesh.face_triangles(|candidate| candidate == face).collect();

                moments.add(&Moments::of_triangles(&triangles, frame.reference));
                meshed_faces.push(face);
            }
            (None, None) => meshed_faces.push(face),
        }
    }
    Ok(SolidMass {
        properties: moments.about(frame.reference),
        face_areas,
        meshed_faces,
    })
}

pub fn face_area(solid: &Solid, face: FaceId) -> Result<Option<f64>, MeasureError> {
    let frame = Frame::of(solid);
    Ok(face_moments(solid, face, &frame)?.map(|moments| moments.area))
}

pub(crate) fn faces_volume(
    solid: &Solid,
    faces: &[FaceId],
    bounds: Aabb,
) -> Result<Option<f64>, MeasureError> {
    let frame = Frame::around(Some(bounds));
    let mut volume = 0.0;

    for face in faces {
        interrupt::check()?;

        let Some(moments) = face_moments(solid, *face, &frame)? else {
            return Ok(None);
        };

        volume += moments.volume;
    }
    Ok(Some(volume))
}

struct Frame {
    reference: Point3,
    scales: Values,
    area: f64,
    boundary: Tolerance,
}

impl Frame {
    fn of(solid: &Solid) -> Self {
        Self::around(solid.outline_box())
    }

    fn around(bounds: Option<Aabb>) -> Self {
        let reference = bounds.map_or(Point3::ZERO, |bounds| bounds.center());
        let reach = bounds
            .map_or(1.0, |bounds| 0.5 * bounds.diagonal())
            .max(LINEAR_RESOLUTION);
        let (length, square, cube) = (reach, reach * reach, reach * reach * reach);

        let scales = [
            1.0, length, square, square, square, cube, cube, cube, cube, cube, cube,
        ];
        Self {
            reference,
            scales,
            area: square,
            boundary: Tolerance {
                relative: BOUNDARY_TOLERANCE,
                scales,
                floor: scaled(&scales, BOUNDARY_TOLERANCE * square),
                noise: BOUNDARY_NOISE,
            },
        }
    }

    fn strip_tolerance(&self, v_range: Interval) -> Tolerance {
        let per_v = STRIP_TOLERANCE * self.area / v_range.length().max(f64::MIN_POSITIVE);
        Tolerance {
            relative: STRIP_TOLERANCE,
            scales: self.scales,
            floor: scaled(&self.scales, per_v),
            noise: 0.0,
        }
    }
}

fn face_moments(
    solid: &Solid,
    face: FaceId,
    frame: &Frame,
) -> Result<Option<Moments>, MeasureError> {
    let definition = solid.face(face).ok_or(MeasureError::MissingFace(face))?;
    let surface = definition.surface();
    let mut loops: Vec<Vec<&Coedge>> = Vec::new();

    for loop_id in definition.loops() {
        let face_loop = solid
            .face_loop(*loop_id)
            .ok_or(MeasureError::MissingFace(face))?;
        let found = face_loop
            .coedges()
            .iter()
            .map(|coedge| solid.coedge(*coedge).ok_or(MeasureError::MissingFace(face)))
            .collect::<Result<Vec<_>, _>>()?;

        loops.push(found);
    }

    let coedges: Vec<&Coedge> = loops.iter().flatten().copied().collect();

    let Some((u_range, v_range)) = uv_ranges(&coedges) else {
        return Ok(None);
    };

    let across = strip_parameter(solid, surface, &coedges, (u_range, v_range));
    let (strip_range, along_range) = match across {
        Parameter::U => (u_range, v_range),
        Parameter::V => (v_range, u_range),
    };

    let domain = Domain {
        surface,
        across,
        middle: strip_range.middle(),
        strip: frame.strip_tolerance(along_range),
        u_kinks: surface_u_kinks(surface, u_range),
        v_kinks: surface_v_kinks(surface, v_range),
    };
    let budget = Budget::new(FACE_EVALUATIONS);
    let mut total = [0.0; COMPONENTS];

    for coedge in &coedges {
        interrupt::check()?;

        if is_level(solid, coedge, across.other(), along_range) {
            continue;
        }

        let found = boundary_integral(solid, &domain, coedge, frame, &budget)?;

        if !found.converged {
            return Ok(None);
        }
        quadrature::add(&mut total, &found.values, 1.0);
    }

    let reach = GAP_LIMIT * u_range.length().max(v_range.length());

    for face_loop in &loops {
        for (from, to) in face_loop.iter().zip(face_loop.iter().cycle().skip(1)) {
            let end = coedge_end(solid, surface, from, End::Last)
                .ok_or(MeasureError::MissingEdge(from.edge()))?;
            let start = coedge_end(solid, surface, to, End::First)
                .ok_or(MeasureError::MissingEdge(to.edge()))?;

            let found = gap_integral(&domain, (end, start), reach, frame.reference, &budget);

            if !found.converged {
                return Ok(None);
            }
            quadrature::add(&mut total, &found.values, 1.0);
        }
    }
    Ok(Some(moments_of(&total, definition.sense().sign())))
}

#[derive(Clone, Copy, Debug)]
enum End {
    First,
    Last,
}

fn coedge_end(solid: &Solid, surface: &Surface, coedge: &Coedge, end: End) -> Option<Point2> {
    let edge = solid.edge(coedge.edge())?;
    let range = edge.interval();
    let parameter = match (end, coedge.sense().is_same()) {
        (End::First, true) | (End::Last, false) => range.start(),
        (End::First, false) | (End::Last, true) => range.end(),
    };

    let point = edge.curve().point(parameter);
    Some(surface.project(point, Some(coedge.pcurve().uv_at(parameter))))
}

fn gap_integral(
    domain: &Domain<'_>,
    (from, to): (Point2, Point2),
    reach: f64,
    reference: Point3,
    budget: &Budget,
) -> Quadrature {
    let gap = to - from;

    if gap == Vector2::ZERO || gap.abs().max_element() > reach {
        return Quadrature {
            values: [0.0; COMPONENTS],
            converged: true,
        };
    }

    let middle = from + 0.5 * gap;
    let step = match domain.across {
        Parameter::U => gap.y,
        Parameter::V => -gap.x,
    };
    let found = strip(domain, middle, reference, budget);
    Quadrature {
        values: scaled(&found.values, step),
        converged: found.converged,
    }
}

struct Domain<'a> {
    surface: &'a Surface,
    across: Parameter,
    middle: f64,
    strip: Tolerance,
    u_kinks: Vec<f64>,
    v_kinks: Vec<f64>,
}

fn strip_parameter(
    solid: &Solid,
    surface: &Surface,
    coedges: &[&Coedge],
    (u_range, v_range): (Interval, Interval),
) -> Parameter {
    if !surface.poles().is_empty() {
        return Parameter::U;
    }

    let cost = |across: Parameter, range: Interval, along: Interval| {
        let strip = strip_pieces(surface, across, range)
            .len()
            .saturating_mul(strip_evaluations(surface, across));

        coedges
            .iter()
            .filter(|coedge| !is_level(solid, coedge, across.other(), along))
            .map(|coedge| {
                let pieces = solid.edge(coedge.edge()).map_or(1, |edge| {
                    let range = edge.interval();
                    quadrature::pieces(range, curve_breaks(edge.curve(), range)).len()
                });
                pieces.saturating_mul(1 + strip)
            })
            .fold(0_usize, usize::saturating_add)
    };

    if cost(Parameter::V, v_range, u_range) < cost(Parameter::U, u_range, v_range) {
        Parameter::V
    } else {
        Parameter::U
    }
}

fn is_level(solid: &Solid, coedge: &Coedge, along: Parameter, range: Interval) -> bool {
    let exact = solid
        .edge(coedge.edge())
        .is_some_and(|edge| !matches!(edge.curve(), Curve::Intersection(_)));

    if !exact {
        return false;
    }

    let samples = coedge.pcurve().samples();
    let Some(first) = samples.first().map(|sample| along.of(sample.uv)) else {
        return true;
    };
    let slack = LEVEL_SLACK * range.length();

    samples
        .iter()
        .all(|sample| (along.of(sample.uv) - first).abs() <= slack)
}

fn uv_ranges(coedges: &[&Coedge]) -> Option<(Interval, Interval)> {
    let uvs = coedges
        .iter()
        .flat_map(|coedge| coedge.pcurve().samples())
        .map(|sample| sample.uv);
    let (low, high) = uvs.fold(
        (
            Point2::splat(f64::INFINITY),
            Point2::splat(f64::NEG_INFINITY),
        ),
        |(low, high), uv| (low.min(uv), high.max(uv)),
    );
    Some((Interval::new(low.x, high.x)?, Interval::new(low.y, high.y)?))
}

fn moments_of(total: &Values, sign: f64) -> Moments {
    let [area, volume, x, y, z, xx, xy, xz, yy, yz, zz] = *total;
    Moments {
        area: sign * area,
        volume,
        first: Vector3::new(x, y, z),
        second: [[xx, xy, xz], [xy, yy, yz], [xz, yz, zz]],
    }
}

fn boundary_integral(
    solid: &Solid,
    domain: &Domain<'_>,
    coedge: &Coedge,
    frame: &Frame,
    budget: &Budget,
) -> Result<Quadrature, MeasureError> {
    let edge = solid
        .edge(coedge.edge())
        .ok_or(MeasureError::MissingEdge(coedge.edge()))?;
    let curve = edge.curve();
    let range = edge.interval();
    let pcurve = coedge.pcurve();
    let surface = domain.surface;

    let on_surface = |parameter: f64| {
        let point = curve.evaluate(parameter);
        let uv = surface.project(point.point, Some(pcurve.uv_at(parameter)));
        (point, uv)
    };
    let mut breaks = curve_breaks(curve, range);

    breaks.extend(kink_crossings(domain, coedge, &|parameter| {
        on_surface(parameter).1
    }));

    let mut strips_converged = true;

    let found = quadrature::integrate(
        quadrature::pieces(range, breaks),
        &frame.boundary,
        budget,
        |parameter| {
            let (point, uv) = on_surface(parameter);

            let at = surface.evaluate(uv.x, uv.y);
            let Some(rate) = along_rate(&at, (point.point, point.first), domain.across) else {
                return [0.0; COMPONENTS];
            };
            let strip = strip(domain, uv, frame.reference, budget);

            strips_converged &= strip.converged;
            scaled(&strip.values, rate)
        },
    );
    Ok(Quadrature {
        values: scaled(&found.values, coedge.sense().sign()),
        converged: found.converged && strips_converged,
    })
}

fn kink_crossings(domain: &Domain<'_>, coedge: &Coedge, uv_at: &dyn Fn(f64) -> Point2) -> Vec<f64> {
    if domain.u_kinks.is_empty() && domain.v_kinks.is_empty() {
        return Vec::new();
    }

    let samples: Vec<(f64, Point2)> = coedge
        .pcurve()
        .samples()
        .iter()
        .map(|sample| (sample.parameter, uv_at(sample.parameter)))
        .collect();
    let mut crossings = Vec::new();
    let lines = [
        (Parameter::U, &domain.u_kinks),
        (Parameter::V, &domain.v_kinks),
    ];

    for pair in samples.windows(2) {
        let [(start, start_uv), (end, end_uv)] = pair else {
            continue;
        };
        for (parameter, kinks) in lines {
            let (from, to) = (parameter.of(*start_uv), parameter.of(*end_uv));
            for kink in kinks {
                if crossings.len() >= MAX_CROSSINGS {
                    return crossings;
                }

                let on_line = KINK_SLACK * (1.0 + kink.abs());

                if (from - kink) * (to - kink) < 0.0
                    && (from - kink).abs().max((to - kink).abs()) > on_line
                {
                    let offset = |along: f64| parameter.of(uv_at(along)) - kink;

                    crossings.push(bisect(*start, *end, from - kink, offset));
                }
            }
        }
    }
    crossings
}

#[derive(Clone, Copy, Debug)]
enum Parameter {
    U,
    V,
}

impl Parameter {
    fn of(self, uv: Point2) -> f64 {
        match self {
            Self::U => uv.x,
            Self::V => uv.y,
        }
    }

    fn other(self) -> Self {
        match self {
            Self::U => Self::V,
            Self::V => Self::U,
        }
    }

    fn with(self, uv: Point2, value: f64) -> Point2 {
        match self {
            Self::U => Point2::new(value, uv.y),
            Self::V => Point2::new(uv.x, value),
        }
    }
}

fn bisect(start: f64, end: f64, start_offset: f64, offset: impl Fn(f64) -> f64) -> f64 {
    let (mut low, mut high) = (start, end);

    for _ in 0..MAX_BISECTIONS {
        let middle = 0.5 * (low + high);

        if (high - low).abs() <= BISECTION_EPSILON * (1.0 + middle.abs()) {
            return middle;
        }
        if (offset(middle) > 0.0) == (start_offset > 0.0) {
            low = middle;
        } else {
            high = middle;
        }
    }
    0.5 * (low + high)
}

fn along_rate(
    at: &SurfaceDerivatives,
    (point, tangent): (Point3, Vector3),
    across: Parameter,
) -> Option<f64> {
    let offset = point - at.point;
    let (uu, uv, vv) = (
        at.du.dot(at.du) - offset.dot(at.duu),
        at.du.dot(at.dv) - offset.dot(at.duv),
        at.dv.dot(at.dv) - offset.dot(at.dvv),
    );

    let determinant = uu * vv - uv * uv;

    if determinant.is_nan() || determinant <= 0.0 {
        return None;
    }

    let (along, speed) = match across {
        Parameter::U => (
            (uu * at.dv.dot(tangent) - uv * at.du.dot(tangent)) / determinant,
            at.dv.length(),
        ),
        Parameter::V => (
            (uv * at.dv.dot(tangent) - vv * at.du.dot(tangent)) / determinant,
            at.du.length(),
        ),
    };

    let level = (along * speed).abs() <= LEVEL_RATE * tangent.length();
    (along.is_finite() && !level).then_some(along)
}

fn strip(domain: &Domain<'_>, uv: Point2, reference: Point3, budget: &Budget) -> Quadrature {
    let (surface, middle, across) = (domain.surface, domain.middle, domain.across);
    let end = across.of(uv);

    let (range, sign) = if end >= middle {
        (Interval::new(middle, end), 1.0)
    } else {
        (Interval::new(end, middle), -1.0)
    };
    let Some(range) = range else {
        return Quadrature {
            values: [0.0; COMPONENTS],
            converged: false,
        };
    };

    let pieces = strip_pieces(surface, across, range);
    let integrand = |parameter: f64| {
        let at = across.with(uv, parameter);
        density(&surface.evaluate(at.x, at.y), reference)
    };

    let found = if is_linear_across(surface, across) {
        quadrature::integrate_cubic(pieces, budget, integrand)
    } else {
        quadrature::integrate(pieces, &domain.strip, budget, integrand)
    };
    Quadrature {
        values: scaled(&found.values, sign),
        converged: found.converged,
    }
}

fn strip_pieces(surface: &Surface, across: Parameter, range: Interval) -> Vec<Interval> {
    let breaks = match across {
        Parameter::U => surface_u_breaks(surface, range),
        Parameter::V => surface_v_breaks(surface, range),
    };
    let pieces = quadrature::pieces(range, breaks);

    match surface {
        Surface::BSpline(spline) if is_rational_across(spline, across) => pieces
            .into_iter()
            .flat_map(|piece| {
                [
                    Interval::new(piece.start(), piece.middle()),
                    Interval::new(piece.middle(), piece.end()),
                ]
            })
            .flatten()
            .collect(),
        _ => pieces,
    }
}

fn is_rational_across(spline: &BSplineSurface, across: Parameter) -> bool {
    let Some(weights) = spline.weights() else {
        return false;
    };
    let columns = spline.columns().max(1);

    match across {
        Parameter::U => weights.chunks(columns).any(|row| {
            row.windows(2)
                .any(|pair| matches!(pair, [first, second] if first != second))
        }),
        Parameter::V => (0..columns).any(|column| {
            let mut down = weights.iter().skip(column).step_by(columns);
            down.next()
                .is_some_and(|first| down.any(|weight| weight != first))
        }),
    }
}

fn strip_evaluations(surface: &Surface, across: Parameter) -> usize {
    if is_linear_across(surface, across) {
        quadrature::CUBIC_EVALUATIONS
    } else {
        quadrature::KRONROD_EVALUATIONS
    }
}

fn is_linear_across(surface: &Surface, across: Parameter) -> bool {
    matches!(
        (surface, across),
        (Surface::Plane(_), _) | (Surface::Cylinder(_) | Surface::Extrusion(_), Parameter::V)
    )
}

fn density(at: &SurfaceDerivatives, reference: Point3) -> Values {
    let normal = at.du.cross(at.dv);
    let x = at.point - reference;

    let flux = x.dot(normal);
    let (third, quarter, fifth) = (flux / 3.0, flux / 4.0, flux / 5.0);
    [
        normal.length(),
        third,
        x.x * quarter,
        x.y * quarter,
        x.z * quarter,
        x.x * x.x * fifth,
        x.x * x.y * fifth,
        x.x * x.z * fifth,
        x.y * x.y * fifth,
        x.y * x.z * fifth,
        x.z * x.z * fifth,
    ]
}

pub(crate) fn curve_breaks(curve: &Curve, range: Interval) -> Vec<f64> {
    match curve {
        Curve::Circle(_) | Curve::Ellipse(_) => quadrature::steps(range, QUARTER_TURN).collect(),
        Curve::Line(_) | Curve::BSpline(_) | Curve::Intersection(_) => curve_kinks(curve, range),
    }
}

fn curve_kinks(curve: &Curve, range: Interval) -> Vec<f64> {
    match curve {
        Curve::Line(_) | Curve::Circle(_) | Curve::Ellipse(_) => Vec::new(),
        Curve::BSpline(spline) => spline.breakpoints(),
        Curve::Intersection(intersection) => intersection.seeds(range),
    }
}

fn surface_u_kinks(surface: &Surface, range: Interval) -> Vec<f64> {
    match surface {
        Surface::Extrusion(extrusion) => curve_kinks(extrusion.profile(), range),
        Surface::BSpline(spline) => knot_breaks(
            spline.u_knots(),
            spline.u_domain(),
            spline.u_period(),
            range,
        ),
        Surface::Plane(_)
        | Surface::Cylinder(_)
        | Surface::Cone(_)
        | Surface::Sphere(_)
        | Surface::Torus(_)
        | Surface::Revolution(_) => Vec::new(),
    }
}

fn surface_v_kinks(surface: &Surface, range: Interval) -> Vec<f64> {
    match surface {
        Surface::Revolution(revolution) => curve_kinks(revolution.profile(), range),
        Surface::BSpline(spline) => knot_breaks(
            spline.v_knots(),
            spline.v_domain(),
            spline.v_period(),
            range,
        ),
        Surface::Plane(_)
        | Surface::Cylinder(_)
        | Surface::Cone(_)
        | Surface::Sphere(_)
        | Surface::Torus(_)
        | Surface::Extrusion(_) => Vec::new(),
    }
}

pub(crate) fn surface_u_breaks(surface: &Surface, range: Interval) -> Vec<f64> {
    match surface {
        Surface::Cylinder(_)
        | Surface::Cone(_)
        | Surface::Sphere(_)
        | Surface::Torus(_)
        | Surface::Revolution(_) => quadrature::steps(range, QUARTER_TURN).collect(),
        Surface::Extrusion(extrusion) => curve_breaks(extrusion.profile(), range),
        Surface::Plane(_) | Surface::BSpline(_) => surface_u_kinks(surface, range),
    }
}

pub(crate) fn surface_v_breaks(surface: &Surface, range: Interval) -> Vec<f64> {
    match surface {
        Surface::Sphere(_) | Surface::Torus(_) => quadrature::steps(range, QUARTER_TURN).collect(),
        Surface::Revolution(revolution) => curve_breaks(revolution.profile(), range),
        Surface::Plane(_)
        | Surface::Cylinder(_)
        | Surface::Cone(_)
        | Surface::Extrusion(_)
        | Surface::BSpline(_) => surface_v_kinks(surface, range),
    }
}

fn knot_breaks(knots: &[f64], domain: Interval, period: Option<f64>, range: Interval) -> Vec<f64> {
    let mut inside: Vec<f64> = knots
        .iter()
        .copied()
        .filter(|knot| domain.contains(*knot))
        .collect();

    inside.dedup();
    match period.filter(|period| *period > 0.0) {
        Some(period) => {
            let shifts = |knot: f64| {
                let low = ((range.start() - knot) / period).floor();
                let high = ((range.end() - knot) / period).ceil();
                quadrature::steps(Interval::new(low, high).unwrap_or(Interval::UNIT), 1.0)
                    .map(move |turns| knot + turns * period)
            };
            inside.into_iter().flat_map(shifts).collect()
        }
        None => inside,
    }
}
