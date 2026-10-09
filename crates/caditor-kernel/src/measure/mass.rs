use std::{collections::BTreeMap, f64::consts::FRAC_PI_2};

use caditor_geometry::{Point2, Point3, Vector3};

use super::{
    MeasureError,
    quadrature::{self, Budget, COMPONENTS, Quadrature, Tolerance, Values, scaled},
};
use crate::{
    curve::Curve,
    interrupt,
    interval::Interval,
    surface::{Surface, SurfaceDerivatives},
    tessellation::{MassProperties, Mesh, Moments},
    tolerance::LINEAR_RESOLUTION,
    topology::{Coedge, FaceId, Solid},
};

const BOUNDARY_TOLERANCE: f64 = 1e-12;
const STRIP_TOLERANCE: f64 = 1e-14;
const FACE_EVALUATIONS: usize = 1 << 23;
const QUARTER_TURN: f64 = FRAC_PI_2;
const LEVEL_RATE: f64 = 1e-15;
const MAX_CROSSINGS: usize = 4096;
const MAX_BISECTIONS: usize = 80;
const BISECTION_EPSILON: f64 = 1e-15;

#[derive(Debug, Clone, PartialEq)]
pub struct SolidMass {
    pub properties: MassProperties,
    pub face_areas: BTreeMap<FaceId, f64>,
    pub meshed_faces: Vec<FaceId>,
}

pub fn mass_properties(solid: &Solid, mesh: &Mesh) -> Result<SolidMass, MeasureError> {
    let frame = Frame::of(solid);
    let mut moments = Moments::default();
    let mut face_areas = BTreeMap::new();
    let mut meshed_faces = Vec::new();
    for (face, _) in solid.faces() {
        interrupt::check()?;
        match face_moments(solid, face, &frame)? {
            Some(integrated) => {
                moments.add(&integrated);
                face_areas.insert(face, integrated.area);
            }
            None => {
                let triangles: Vec<[Point3; 3]> =
                    mesh.face_triangles(|candidate| candidate == face).collect();
                moments.add(&Moments::of_triangles(&triangles, frame.reference));
                meshed_faces.push(face);
            }
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

struct Frame {
    reference: Point3,
    scales: Values,
    area: f64,
    boundary: Tolerance,
}

impl Frame {
    fn of(solid: &Solid) -> Self {
        let bounds = solid.outline_box();
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
            },
        }
    }

    fn strip_tolerance(&self, v_range: Interval) -> Tolerance {
        let per_v = STRIP_TOLERANCE * self.area / v_range.length().max(f64::MIN_POSITIVE);
        Tolerance {
            relative: STRIP_TOLERANCE,
            scales: self.scales,
            floor: scaled(&self.scales, per_v),
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
    let mut coedges: Vec<&Coedge> = Vec::new();
    for loop_id in definition.loops() {
        let face_loop = solid
            .face_loop(*loop_id)
            .ok_or(MeasureError::MissingFace(face))?;
        for coedge in face_loop.coedges() {
            coedges.push(
                solid
                    .coedge(*coedge)
                    .ok_or(MeasureError::MissingFace(face))?,
            );
        }
    }
    let Some((u_range, v_range)) = uv_ranges(&coedges) else {
        return Ok(None);
    };
    let domain = Domain {
        surface,
        middle: u_range.middle(),
        strip: frame.strip_tolerance(v_range),
        u_kinks: surface_u_kinks(surface, u_range),
        v_kinks: surface_v_kinks(surface, v_range),
    };
    let budget = Budget::new(FACE_EVALUATIONS);
    let mut total = [0.0; COMPONENTS];
    for coedge in coedges {
        interrupt::check()?;
        let found = boundary_integral(solid, &domain, coedge, frame, &budget)?;
        if !found.converged {
            return Ok(None);
        }
        quadrature::add(&mut total, &found.values, 1.0);
    }
    Ok(Some(moments_of(&total, definition.sense().sign())))
}

struct Domain<'a> {
    surface: &'a Surface,
    middle: f64,
    strip: Tolerance,
    u_kinks: Vec<f64>,
    v_kinks: Vec<f64>,
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
            let Some(rate) = v_rate(&surface.evaluate(uv.x, uv.y), point.first) else {
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
                if (from - kink) * (to - kink) < 0.0 {
                    let offset = |along: f64| parameter.of(uv_at(along)) - kink;
                    crossings.push(bisect(*start, *end, from - kink, offset));
                }
            }
        }
    }
    crossings
}

#[derive(Debug, Clone, Copy)]
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

fn v_rate(at: &SurfaceDerivatives, tangent: Vector3) -> Option<f64> {
    let (uu, uv, vv) = (at.du.dot(at.du), at.du.dot(at.dv), at.dv.dot(at.dv));
    let determinant = uu * vv - uv * uv;
    if determinant.is_nan() || determinant <= 0.0 {
        return None;
    }
    let rate = (uu * at.dv.dot(tangent) - uv * at.du.dot(tangent)) / determinant;
    let level = (rate * vv.sqrt()).abs() <= LEVEL_RATE * tangent.length();
    (rate.is_finite() && !level).then_some(rate)
}

fn strip(domain: &Domain<'_>, uv: Point2, reference: Point3, budget: &Budget) -> Quadrature {
    let (surface, middle) = (domain.surface, domain.middle);
    let (range, sign) = if uv.x >= middle {
        (Interval::new(middle, uv.x), 1.0)
    } else {
        (Interval::new(uv.x, middle), -1.0)
    };
    let Some(range) = range else {
        return Quadrature {
            values: [0.0; COMPONENTS],
            converged: false,
        };
    };
    let found = quadrature::integrate(
        quadrature::pieces(range, surface_u_breaks(surface, range)),
        &domain.strip,
        budget,
        |u| density(&surface.evaluate(u, uv.y), reference),
    );
    Quadrature {
        values: scaled(&found.values, sign),
        converged: found.converged,
    }
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
