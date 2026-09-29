use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    f64::consts::FRAC_PI_2,
    rc::Rc,
};

use caditor_geometry::{Plane, Point3, Vector3};
use caditor_kernel::{
    BSpline, BSplineSurface, Circle, Cone, Curve, Cylinder, Ellipse, Extrusion, GeometryError,
    Interval, LINEAR_RESOLUTION, Line, MAX_SPLINE_DEGREE, PlaneSurface, Revolution, Sphere,
    Surface, Torus,
};

use crate::read::{
    graph::{Entity, Graph, Problem, Read, friendly},
    spline::{Homogeneous, bezier_knots, clamp, expand_knots, knot_count, uniform_knots},
    units::Units,
};

const MAX_CURVE_DEPTH: usize = 8;
const PIECE_SAMPLES: usize = 64;
const OFFSET_SAMPLES: usize = 256;
const JOINT_GAP: f64 = 1e-6;
pub(crate) const MAX_WORK: usize = 4_000_000;

fn sampled(curve: &Curve, range: Interval) -> Vec<Point3> {
    range
        .split(PIECE_SAMPLES)
        .map(|parameter| curve.point(parameter))
        .collect()
}

fn polyline(id: u64, points: Vec<Point3>) -> Read<Curve> {
    let mut knots = vec![0.0, 0.0];
    let mut travelled = 0.0;
    for pair in points.windows(2) {
        if let [a, b] = pair {
            travelled += a.distance(*b);
            knots.push(travelled);
        }
    }
    knots.push(travelled);
    BSpline::new(1, knots, points)
        .map(Curve::BSpline)
        .map_err(|error| Problem::new(id, format!("is not a usable curve ({error})")))
}

fn spline_degree(value: i64, id: u64) -> Read<usize> {
    let degree = usize::try_from(value).map_err(|_| Problem::new(id, "has a negative degree"))?;
    if degree > MAX_SPLINE_DEGREE {
        return Err(Problem::new(
            id,
            format!(
                "has degree {degree}, and caditor reads splines up to degree {MAX_SPLINE_DEGREE}"
            ),
        ));
    }
    Ok(degree)
}

#[derive(Debug, Clone)]
pub(crate) struct Work(Rc<Cell<usize>>);

impl Work {
    pub fn new(budget: usize) -> Self {
        Self(Rc::new(Cell::new(budget)))
    }

    pub fn charge(&self, id: u64) -> Read<()> {
        let left = self.0.get();
        if left == 0 {
            return Err(Problem::new(
                id,
                "is part of a model too intricate to import in one go",
            ));
        }
        self.0.set(left - 1);
        Ok(())
    }
}

pub(crate) struct Geometry<'a> {
    pub graph: Graph<'a>,
    pub units: Units,
    work: Work,
    curves: RefCell<BTreeMap<u64, Read<Curve>>>,
    surfaces: RefCell<BTreeMap<u64, Read<Surface>>>,
}

impl<'a> Geometry<'a> {
    pub fn new(graph: Graph<'a>, units: Units, work: Work) -> Self {
        Self {
            graph,
            units,
            work,
            curves: RefCell::new(BTreeMap::new()),
            surfaces: RefCell::new(BTreeMap::new()),
        }
    }

    pub fn charge(&self, id: u64) -> Read<()> {
        self.work.charge(id)
    }

    pub fn point(&self, id: u64) -> Read<Point3> {
        let fields = self.graph.entity(id)?.record("CARTESIAN_POINT")?;
        let coordinates = fields.reals(1)?;
        let coordinate = |index: usize| coordinates.get(index).copied().unwrap_or(0.0);
        if coordinates.len() < 2 {
            return Err(Problem::new(id, "is not a point in space"));
        }
        Ok(Point3::new(coordinate(0), coordinate(1), coordinate(2)) * self.units.length)
    }

    pub fn direction(&self, id: u64) -> Read<Vector3> {
        let fields = self.graph.entity(id)?.record("DIRECTION")?;
        let ratios = fields.reals(1)?;
        let ratio = |index: usize| ratios.get(index).copied().unwrap_or(0.0);
        Vector3::new(ratio(0), ratio(1), ratio(2))
            .try_normalize()
            .ok_or_else(|| Problem::new(id, "is a direction of no length"))
    }

    fn vector_direction(&self, id: u64) -> Read<Vector3> {
        let fields = self.graph.entity(id)?.record("VECTOR")?;
        let direction = self.direction(fields.reference(1)?)?;
        let magnitude = fields.real(2)?;
        if magnitude == 0.0 {
            return Err(Problem::new(id, "is a vector of no length"));
        }
        Ok(direction * magnitude.signum())
    }

    pub fn placement(&self, id: u64) -> Read<Plane> {
        let fields = self.graph.entity(id)?.record("AXIS2_PLACEMENT_3D")?;
        let origin = self.point(fields.reference(1)?)?;
        let axis = match fields.optional_reference(2) {
            Some(axis) => self.direction(axis)?,
            None => Vector3::Z,
        };
        let reference = match fields.optional_reference(3) {
            Some(reference) => self.direction(reference)?,
            None => {
                if axis.x.abs() < 0.9 {
                    Vector3::X
                } else {
                    Vector3::Y
                }
            }
        };
        Plane::with_x_axis(origin, axis, reference)
            .ok_or_else(|| Problem::new(id, "has its reference direction along its axis"))
    }

    fn length(&self, fields: &crate::read::graph::Fields<'_>, index: usize) -> Read<f64> {
        Ok(fields.real(index)? * self.units.length)
    }

    pub fn curve(&self, id: u64) -> Read<Curve> {
        self.curve_at(id, 0)
    }

    fn curve_at(&self, id: u64, depth: usize) -> Read<Curve> {
        if let Some(known) = self.curves.borrow().get(&id) {
            return known.clone();
        }
        if depth > MAX_CURVE_DEPTH {
            return Err(Problem::new(id, "refers to itself"));
        }
        self.charge(id)?;
        let built = self.build_curve(id, depth);
        if built.is_ok() || depth == 0 {
            self.curves.borrow_mut().insert(id, built.clone());
        }
        built
    }

    fn build_curve(&self, id: u64, depth: usize) -> Read<Curve> {
        let entity = self.graph.entity(id)?;
        let kernel =
            |error: GeometryError| Problem::new(id, format!("is not a usable curve ({error})"));
        match entity.kind() {
            "LINE" => {
                let fields = entity.record("LINE")?;
                let origin = self.point(fields.reference(1)?)?;
                let direction = self.vector_direction(fields.reference(2)?)?;
                Ok(Line::new(origin, direction).map_err(kernel)?.into())
            }
            "CIRCLE" => {
                let fields = entity.record("CIRCLE")?;
                let frame = self.placement(fields.reference(1)?)?;
                let radius = self.length(&fields, 2)?;
                Ok(Circle::new(frame, radius).map_err(kernel)?.into())
            }
            "ELLIPSE" => {
                let fields = entity.record("ELLIPSE")?;
                let frame = self.placement(fields.reference(1)?)?;
                let major = self.length(&fields, 2)?;
                let minor = self.length(&fields, 3)?;
                Ok(Ellipse::new(frame, major, minor).map_err(kernel)?.into())
            }
            "SURFACE_CURVE" | "SEAM_CURVE" | "INTERSECTION_CURVE" | "BOUNDED_SURFACE_CURVE" => {
                let basis = match entity.fields() {
                    Ok(fields) => fields.reference(1)?,
                    Err(_) => entity.record("SURFACE_CURVE")?.reference(0)?,
                };
                self.curve_at(basis, depth + 1)
            }
            "TRIMMED_CURVE" => {
                let fields = entity.record("TRIMMED_CURVE")?;
                self.curve_at(fields.reference(1)?, depth + 1)
            }
            "OFFSET_CURVE_3D" => self.offset_curve(entity, depth),
            _ if entity.is("COMPOSITE_CURVE") => self.composite_curve(entity, depth),
            "POLYLINE" => {
                let fields = entity.record("POLYLINE")?;
                let points = fields
                    .references(1)?
                    .into_iter()
                    .map(|point| self.point(point))
                    .collect::<Read<Vec<Point3>>>()?;
                let mut knots = vec![0.0];
                let mut travelled = 0.0;
                for pair in points.windows(2) {
                    if let [a, b] = pair {
                        travelled += a.distance(*b);
                        knots.push(travelled);
                    }
                }
                knots.insert(0, 0.0);
                knots.push(travelled);
                Ok(Curve::BSpline(
                    BSpline::new(1, knots, points).map_err(kernel)?,
                ))
            }
            "BEZIER_CURVE" | "UNIFORM_CURVE" | "QUASI_UNIFORM_CURVE" => {
                Ok(Curve::BSpline(self.spline_curve(entity)?))
            }
            _ if entity.is("B_SPLINE_CURVE") || entity.is("B_SPLINE_CURVE_WITH_KNOTS") => {
                Ok(Curve::BSpline(self.spline_curve(entity)?))
            }
            other => Err(Problem::new(
                id,
                format!(
                    "is a {} edge, which caditor cannot import yet",
                    friendly(other)
                ),
            )),
        }
    }

    fn composite_curve(&self, entity: Entity<'_>, depth: usize) -> Read<Curve> {
        let id = entity.id;
        let segments = match entity.fields() {
            Ok(fields) => fields.references(1)?,
            Err(_) => entity.record("COMPOSITE_CURVE")?.references(0)?,
        };
        let mut points: Vec<Point3> = Vec::new();
        for segment in segments {
            let fields = self.graph.entity(segment)?.fields()?;
            let same_sense = fields.logical(1)?;
            let mut piece = self.piece(fields.reference(2)?, depth + 1)?;
            if !same_sense {
                piece.reverse();
            }
            let joined = points
                .last()
                .zip(piece.first())
                .is_some_and(|(last, first)| {
                    last.distance(*first) <= JOINT_GAP * self.units.length
                });
            points.extend(piece.into_iter().skip(usize::from(joined)));
        }
        polyline(id, points)
    }

    fn piece(&self, id: u64, depth: usize) -> Read<Vec<Point3>> {
        if depth > MAX_CURVE_DEPTH {
            return Err(Problem::new(id, "refers to itself"));
        }
        self.charge(id)?;
        let entity = self.graph.entity(id)?;
        if entity.kind() != "TRIMMED_CURVE" {
            let curve = self.curve_at(id, depth)?;
            let range = curve
                .domain()
                .bounded()
                .ok_or_else(|| Problem::new(id, "is a piece of a curve with no end"))?;
            return Ok(sampled(&curve, range));
        }
        let fields = entity.record("TRIMMED_CURVE")?;
        let basis_id = fields.reference(1)?;
        let curve = self.curve_at(basis_id, depth + 1)?;
        let scale = self.parameter_scale(basis_id)?;
        let trim = |index: usize| -> Read<f64> {
            let options = fields.list(index)?;
            let by_point = options
                .iter()
                .filter_map(crate::part21::Parameter::reference)
                .find_map(|point| self.point(point).ok());
            let search = curve
                .domain()
                .bounded()
                .or_else(|| curve.period().and_then(|period| Interval::new(0.0, period)))
                .unwrap_or_else(|| Interval::new(-1e12, 1e12).unwrap_or(Interval::UNIT));
            match by_point {
                Some(point) => Ok(curve.closest_parameter(point, search)),
                None => options
                    .iter()
                    .find_map(crate::part21::Parameter::real)
                    .map(|value| value * scale)
                    .ok_or_else(|| Problem::new(id, "is trimmed by nothing it can read")),
            }
        };
        let (first, second) = (trim(2)?, trim(3)?);
        let forward = fields.logical(4)?;
        let (low, mut high) = if forward {
            (first, second)
        } else {
            (second, first)
        };
        if let Some(period) = curve.period() {
            high = low + (high - low).rem_euclid(period);
            if high - low <= period * 1e-12 {
                high = low + period;
            }
        }
        let range =
            Interval::new(low, high).ok_or_else(|| Problem::new(id, "is trimmed to nothing"))?;
        let mut points = sampled(&curve, range);
        if !forward {
            points.reverse();
        }
        Ok(points)
    }

    fn parameter_scale(&self, basis: u64) -> Read<f64> {
        let entity = self.graph.entity(basis)?;
        Ok(match entity.kind() {
            "CIRCLE" | "ELLIPSE" => self.units.angle,
            "LINE" => {
                let vector = entity.record("LINE")?.reference(2)?;
                let magnitude = self.graph.entity(vector)?.record("VECTOR")?.real(2)?;
                magnitude.abs() * self.units.length
            }
            _ => 1.0,
        })
    }

    fn offset_curve(&self, entity: Entity<'_>, depth: usize) -> Read<Curve> {
        let id = entity.id;
        let fields = entity.record("OFFSET_CURVE_3D")?;
        let basis = self.curve_at(fields.reference(1)?, depth + 1)?;
        let distance = self.length(&fields, 2)?;
        let reference = self.direction(fields.reference(4)?)?;
        let offset = |parameter: f64| {
            let tangent = basis.evaluate(parameter).first;
            tangent
                .cross(reference)
                .try_normalize()
                .map(|side| basis.point(parameter) + side * distance)
        };
        if let Curve::Line(line) = &basis {
            let shifted =
                offset(0.0).ok_or_else(|| Problem::new(id, "is offset along its own direction"))?;
            return Ok(Line::new(shifted, line.direction())
                .map_err(|error| Problem::new(id, format!("is not a usable curve ({error})")))?
                .into());
        }
        let range = basis
            .domain()
            .bounded()
            .or_else(|| basis.period().and_then(|period| Interval::new(0.0, period)))
            .ok_or_else(|| Problem::new(id, "offsets a curve with no end"))?;
        let points = range
            .split(OFFSET_SAMPLES)
            .map(|parameter| {
                offset(parameter)
                    .ok_or_else(|| Problem::new(id, "is offset along its own direction somewhere"))
            })
            .collect::<Read<Vec<Point3>>>()?;
        polyline(id, points)
    }

    fn spline_curve(&self, entity: Entity<'_>) -> Read<BSpline<Point3>> {
        let id = entity.id;
        let mismatched = || Problem::new(id, "has knots that do not match");
        let (degree, points, knots) = match entity.fields() {
            Ok(fields) => {
                let degree = spline_degree(fields.integer(1)?, id)?;
                let points = fields.references(2)?;
                let knots = match entity.kind() {
                    "B_SPLINE_CURVE_WITH_KNOTS" => {
                        let multiplicities: Vec<i64> = fields
                            .list(6)?
                            .iter()
                            .filter_map(crate::part21::Parameter::integer)
                            .collect();
                        let expected = knot_count(points.len(), degree).ok_or_else(mismatched)?;
                        expand_knots(&multiplicities, &fields.reals(7)?, expected)
                            .ok_or_else(mismatched)?
                    }
                    "UNIFORM_CURVE" => uniform_knots(points.len(), degree, false),
                    "BEZIER_CURVE" => bezier_knots(points.len(), degree),
                    _ => uniform_knots(points.len(), degree, true),
                };
                (degree, points, knots)
            }
            Err(_) => {
                let curve = entity.record("B_SPLINE_CURVE")?;
                let degree = spline_degree(curve.integer(0)?, id)?;
                let points = curve.references(1)?;
                let knots = if let Ok(with_knots) = entity.record("B_SPLINE_CURVE_WITH_KNOTS") {
                    let multiplicities: Vec<i64> = with_knots
                        .list(0)?
                        .iter()
                        .filter_map(crate::part21::Parameter::integer)
                        .collect();
                    let expected = knot_count(points.len(), degree).ok_or_else(mismatched)?;
                    expand_knots(&multiplicities, &with_knots.reals(1)?, expected)
                        .ok_or_else(mismatched)?
                } else if entity.is("BEZIER_CURVE") {
                    bezier_knots(points.len(), degree)
                } else {
                    uniform_knots(points.len(), degree, !entity.is("UNIFORM_CURVE"))
                };
                (degree, points, knots)
            }
        };
        let weights = entity
            .record("RATIONAL_B_SPLINE_CURVE")
            .ok()
            .map(|rational| rational.reals(0))
            .transpose()?;
        let points = points
            .into_iter()
            .map(|point| self.point(point))
            .collect::<Read<Vec<Point3>>>()?;
        let homogeneous = homogeneous(&points, weights.as_deref())
            .ok_or_else(|| Problem::new(id, "has weights that do not match its points"))?;
        let (knots, homogeneous) = clamp(degree, &knots, &homogeneous)
            .ok_or_else(|| Problem::new(id, "has knots that do not fit its points"))?;
        let (points, weights) = cartesian(&homogeneous)
            .ok_or_else(|| Problem::new(id, "has weights that are not positive"))?;
        let built = if weights.iter().all(|weight| *weight == 1.0) {
            BSpline::new(degree, knots, points)
        } else {
            BSpline::rational(degree, knots, points, weights)
        };
        built.map_err(|error| Problem::new(id, format!("is not a usable curve ({error})")))
    }

    pub fn surface(&self, id: u64) -> Read<Surface> {
        self.surface_at(id, 0)
    }

    fn surface_at(&self, id: u64, depth: usize) -> Read<Surface> {
        if let Some(known) = self.surfaces.borrow().get(&id) {
            return known.clone();
        }
        if depth > MAX_CURVE_DEPTH {
            return Err(Problem::new(id, "refers to itself"));
        }
        self.charge(id)?;
        let built = self.build_surface(id, depth);
        if built.is_ok() || depth == 0 {
            self.surfaces.borrow_mut().insert(id, built.clone());
        }
        built
    }

    fn build_surface(&self, id: u64, depth: usize) -> Read<Surface> {
        let entity = self.graph.entity(id)?;
        let kernel =
            |error: GeometryError| Problem::new(id, format!("is not a usable surface ({error})"));
        match entity.kind() {
            "PLANE" => {
                let fields = entity.record("PLANE")?;
                let frame = self.placement(fields.reference(1)?)?;
                Ok(PlaneSurface::new(frame).map_err(kernel)?.into())
            }
            "CYLINDRICAL_SURFACE" => {
                let fields = entity.record("CYLINDRICAL_SURFACE")?;
                let frame = self.placement(fields.reference(1)?)?;
                let radius = self.length(&fields, 2)?;
                Ok(Cylinder::new(frame, radius).map_err(kernel)?.into())
            }
            "CONICAL_SURFACE" => {
                let fields = entity.record("CONICAL_SURFACE")?;
                let frame = self.placement(fields.reference(1)?)?;
                let radius = self.length(&fields, 2)?;
                let angle = fields.real(3)? * self.units.angle;
                Ok(Cone::new(frame, radius, angle).map_err(kernel)?.into())
            }
            "SPHERICAL_SURFACE" => {
                let fields = entity.record("SPHERICAL_SURFACE")?;
                let frame = self.placement(fields.reference(1)?)?;
                let radius = self.length(&fields, 2)?;
                Ok(Sphere::new(frame, radius).map_err(kernel)?.into())
            }
            "TOROIDAL_SURFACE" => {
                let fields = entity.record("TOROIDAL_SURFACE")?;
                let frame = self.placement(fields.reference(1)?)?;
                let major = self.length(&fields, 2)?;
                let minor = self.length(&fields, 3)?;
                Ok(Torus::new(frame, major, minor).map_err(kernel)?.into())
            }
            "DEGENERATE_TOROIDAL_SURFACE" => {
                let fields = entity.record("DEGENERATE_TOROIDAL_SURFACE")?;
                let frame = self.placement(fields.reference(1)?)?;
                let major = self.length(&fields, 2)?;
                let minor = self.length(&fields, 3)?;
                if minor < major - LINEAR_RESOLUTION {
                    return Ok(Torus::new(frame, major, minor).map_err(kernel)?.into());
                }
                if minor <= major + LINEAR_RESOLUTION {
                    return Err(Problem::new(
                        id,
                        "is a torus whose tube just touches its axis, which caditor cannot import yet",
                    ));
                }
                let outer = fields.logical(4)?;
                Ok(spindle(&frame, major, minor, outer).map_err(kernel)?.into())
            }
            "SURFACE_OF_LINEAR_EXTRUSION" => {
                let fields = entity.record("SURFACE_OF_LINEAR_EXTRUSION")?;
                let profile = self.curve(fields.reference(1)?)?;
                let direction = self.vector_direction(fields.reference(2)?)?;
                Ok(Extrusion::new(profile, direction).map_err(kernel)?.into())
            }
            "SURFACE_OF_REVOLUTION" => {
                let fields = entity.record("SURFACE_OF_REVOLUTION")?;
                let profile = self.curve(fields.reference(1)?)?;
                let axis = self
                    .graph
                    .entity(fields.reference(2)?)?
                    .record("AXIS1_PLACEMENT")?;
                let origin = self.point(axis.reference(1)?)?;
                let direction = match axis.optional_reference(2) {
                    Some(direction) => self.direction(direction)?,
                    None => Vector3::Z,
                };
                Ok(Revolution::new(profile, origin, direction)
                    .map_err(kernel)?
                    .into())
            }
            "BEZIER_SURFACE" | "UNIFORM_SURFACE" | "QUASI_UNIFORM_SURFACE" => {
                Ok(Surface::BSpline(self.spline_surface(entity)?))
            }
            _ if entity.is("B_SPLINE_SURFACE") || entity.is("B_SPLINE_SURFACE_WITH_KNOTS") => {
                Ok(Surface::BSpline(self.spline_surface(entity)?))
            }
            "RECTANGULAR_TRIMMED_SURFACE" => {
                let fields = entity.record("RECTANGULAR_TRIMMED_SURFACE")?;
                self.surface_at(fields.reference(1)?, depth + 1)
            }
            other => Err(Problem::new(
                id,
                format!("is a {}, which caditor cannot import yet", friendly(other)),
            )),
        }
    }
}

fn spindle(
    frame: &Plane,
    major: f64,
    minor: f64,
    outer: bool,
) -> Result<Revolution, GeometryError> {
    let (center, reach) = if outer {
        (major, (-major / minor).acos())
    } else {
        (-major, (major / minor).acos())
    };
    let center = frame.origin() + frame.x_axis() * center;
    let point =
        |angle: f64| center + (frame.x_axis() * angle.cos() + frame.normal() * angle.sin()) * minor;
    let range = Interval::new(-reach, reach).ok_or(GeometryError::NonFinite)?;
    let pieces = (range.length() / FRAC_PI_2).ceil().max(1.0) as usize;
    let step = range.length() / pieces as f64;
    let weight = (0.5 * step).cos();
    let mut points = Vec::with_capacity(2 * pieces + 1);
    let mut weights = Vec::with_capacity(2 * pieces + 1);
    let mut knots = vec![0.0; 3];
    for index in 0..pieces {
        let angle = range.start() + step * index as f64;
        points.push(point(angle));
        weights.push(1.0);
        points.push(center + (point(angle + 0.5 * step) - center) / weight);
        weights.push(weight);
        if index > 0 {
            let knot = index as f64 / pieces as f64;
            knots.extend([knot, knot]);
        }
    }
    points.push(point(range.end()));
    weights.push(1.0);
    knots.extend([1.0; 3]);
    let profile = BSpline::rational(2, knots, points, weights)?;
    Revolution::new(Curve::BSpline(profile), frame.origin(), frame.normal())
}

impl Geometry<'_> {
    fn spline_surface(&self, entity: Entity<'_>) -> Read<BSplineSurface> {
        let id = entity.id;
        let degree = |value: i64| spline_degree(value, id);
        let (u_degree, v_degree, grid_index, fields, knot_fields, knot_offset) =
            match entity.fields() {
                Ok(fields) => (
                    degree(fields.integer(1)?)?,
                    degree(fields.integer(2)?)?,
                    3,
                    fields,
                    Some(fields),
                    8,
                ),
                Err(_) => {
                    let fields = entity.record("B_SPLINE_SURFACE")?;
                    let knots = entity.record("B_SPLINE_SURFACE_WITH_KNOTS").ok();
                    (
                        degree(fields.integer(0)?)?,
                        degree(fields.integer(1)?)?,
                        2,
                        fields,
                        knots,
                        0,
                    )
                }
            };
        let rows = fields.list(grid_index)?;
        let grid: Vec<Vec<u64>> = rows
            .iter()
            .map(|row| {
                row.list()
                    .ok_or_else(|| Problem::new(id, "has a row of points that is not a list"))
                    .and_then(|items| crate::read::graph::references(items, id))
            })
            .collect::<Read<_>>()?;
        let columns = grid.len();
        let count = grid.first().map_or(0, Vec::len);
        if count == 0 || grid.iter().any(|row| row.len() != count) {
            return Err(Problem::new(id, "has rows of points of different lengths"));
        }
        let kind = entity.kind();
        let explicit_knots =
            kind == "B_SPLINE_SURFACE_WITH_KNOTS" || entity.is("B_SPLINE_SURFACE_WITH_KNOTS");
        let (u_knots, v_knots) = match (explicit_knots, knot_fields) {
            (true, Some(knots)) => {
                let integers = |index: usize| -> Read<Vec<i64>> {
                    Ok(knots
                        .list(index)?
                        .iter()
                        .filter_map(crate::part21::Parameter::integer)
                        .collect())
                };
                let mismatched = || Problem::new(id, "has knots that do not match");
                let u_expected = knot_count(columns, u_degree).ok_or_else(mismatched)?;
                let v_expected = knot_count(count, v_degree).ok_or_else(mismatched)?;
                let u = expand_knots(
                    &integers(knot_offset)?,
                    &knots.reals(knot_offset + 2)?,
                    u_expected,
                );
                let v = expand_knots(
                    &integers(knot_offset + 1)?,
                    &knots.reals(knot_offset + 3)?,
                    v_expected,
                );
                match (u, v) {
                    (Some(u), Some(v)) => (u, v),
                    _ => return Err(Problem::new(id, "has knots that do not match")),
                }
            }
            _ if entity.is("BEZIER_SURFACE") => (
                bezier_knots(columns, u_degree),
                bezier_knots(count, v_degree),
            ),
            _ => {
                let clamped = !entity.is("UNIFORM_SURFACE");
                (
                    uniform_knots(columns, u_degree, clamped),
                    uniform_knots(count, v_degree, clamped),
                )
            }
        };
        let weights: Option<Vec<Vec<f64>>> = match entity.record("RATIONAL_B_SPLINE_SURFACE") {
            Ok(rational) => Some(
                rational
                    .list(0)?
                    .iter()
                    .map(|row| {
                        row.list()
                            .ok_or_else(|| Problem::new(id, "has weights that are not a list"))?
                            .iter()
                            .map(|weight| {
                                weight.real().ok_or_else(|| {
                                    Problem::new(id, "has a weight that cannot be read")
                                })
                            })
                            .collect::<Read<Vec<f64>>>()
                    })
                    .collect::<Read<_>>()?,
            ),
            Err(_) => None,
        };
        let mut rows_by_v: Vec<Vec<Homogeneous>> = vec![Vec::with_capacity(columns); count];
        for (u_index, row) in grid.iter().enumerate() {
            for (v_index, point) in row.iter().enumerate() {
                let point = self.point(*point)?;
                let weight = match &weights {
                    Some(weights) => *weights
                        .get(u_index)
                        .and_then(|row| row.get(v_index))
                        .ok_or_else(|| {
                            Problem::new(id, "has weights that do not match its points")
                        })?,
                    None => 1.0,
                };
                if weight.is_nan() || weight <= 0.0 {
                    return Err(Problem::new(id, "has weights that are not positive"));
                }
                if let Some(target) = rows_by_v.get_mut(v_index) {
                    target.push([point.x * weight, point.y * weight, point.z * weight, weight]);
                }
            }
        }
        let unusable = || Problem::new(id, "has knots that do not fit its points");
        let mut u_clamped_knots = None;
        let mut clamped_rows = Vec::with_capacity(rows_by_v.len());
        for row in &rows_by_v {
            let (knots, row) = clamp(u_degree, &u_knots, row).ok_or_else(unusable)?;
            u_clamped_knots = Some(knots);
            clamped_rows.push(row);
        }
        let u_knots = u_clamped_knots.ok_or_else(unusable)?;
        let columns = clamped_rows.first().map_or(0, Vec::len);
        let mut clamped_columns = Vec::with_capacity(columns);
        let mut v_clamped_knots = None;
        for column in 0..columns {
            let values: Vec<Homogeneous> = clamped_rows
                .iter()
                .filter_map(|row| row.get(column).copied())
                .collect();
            let (knots, values) = clamp(v_degree, &v_knots, &values).ok_or_else(unusable)?;
            v_clamped_knots = Some(knots);
            clamped_columns.push(values);
        }
        let v_knots = v_clamped_knots.ok_or_else(unusable)?;
        let rows = clamped_columns.first().map_or(0, Vec::len);
        let mut homogeneous_points = Vec::with_capacity(rows * columns);
        for row in 0..rows {
            for column in &clamped_columns {
                homogeneous_points.push(*column.get(row).ok_or_else(unusable)?);
            }
        }
        let (points, weights) = cartesian(&homogeneous_points)
            .ok_or_else(|| Problem::new(id, "has weights that are not positive"))?;
        let weights = weights
            .iter()
            .any(|weight| *weight != 1.0)
            .then_some(weights);
        BSplineSurface::new(
            u_degree, v_degree, u_knots, v_knots, columns, points, weights,
        )
        .map_err(|error| Problem::new(id, format!("is not a usable surface ({error})")))
    }
}

pub(crate) fn homogeneous(points: &[Point3], weights: Option<&[f64]>) -> Option<Vec<Homogeneous>> {
    match weights {
        None => Some(
            points
                .iter()
                .map(|point| [point.x, point.y, point.z, 1.0])
                .collect(),
        ),
        Some(weights) if weights.len() == points.len() => Some(
            points
                .iter()
                .zip(weights)
                .map(|(point, weight)| {
                    [
                        point.x * weight,
                        point.y * weight,
                        point.z * weight,
                        *weight,
                    ]
                })
                .collect(),
        ),
        Some(_) => None,
    }
}

pub(crate) fn cartesian(points: &[Homogeneous]) -> Option<(Vec<Point3>, Vec<f64>)> {
    let mut cartesian = Vec::with_capacity(points.len());
    let mut weights = Vec::with_capacity(points.len());
    for [x, y, z, weight] in points {
        if weight.is_nan() || *weight <= 0.0 {
            return None;
        }
        cartesian.push(Point3::new(x / weight, y / weight, z / weight));
        weights.push(*weight);
    }
    Some((cartesian, weights))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_degrees_above_the_kernel_limit_are_refused_before_any_work() {
        assert_eq!(spline_degree(3, 7), Ok(3));
        assert_eq!(spline_degree(9, 7), Ok(9));
        let refused = spline_degree(1_000_000_000, 7).unwrap_err();
        assert!(refused.to_string().contains("up to degree 9"), "{refused}");
        assert!(spline_degree(-1, 7).is_err());
    }
}
