use caditor_geometry::{Plane, Point3, Vector3};
use caditor_kernel::{
    BSpline, BSplineSurface, Circle, Cone, Curve, Cylinder, Ellipse, Extrusion, Line,
    MAX_SPLINE_DEGREE, PlaneSurface, Revolution, Sphere, Surface, Torus,
};

use crate::read::{
    graph::{Entity, Graph, Problem, Read, friendly},
    spline::{Homogeneous, clamp, expand_knots, knot_count, uniform_knots},
    units::Units,
};

const MAX_CURVE_DEPTH: usize = 8;

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

pub(crate) struct Geometry<'a> {
    pub graph: Graph<'a>,
    pub units: Units,
}

impl<'a> Geometry<'a> {
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
        if depth > MAX_CURVE_DEPTH {
            return Err(Problem::new(id, "refers to itself"));
        }
        let entity = self.graph.entity(id)?;
        let kernel = |error: caditor_kernel::GeometryError| {
            Problem::new(id, format!("is not a usable curve ({error})"))
        };
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
        if depth > MAX_CURVE_DEPTH {
            return Err(Problem::new(id, "refers to itself"));
        }
        let entity = self.graph.entity(id)?;
        let kernel = |error: caditor_kernel::GeometryError| {
            Problem::new(id, format!("is not a usable surface ({error})"))
        };
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
