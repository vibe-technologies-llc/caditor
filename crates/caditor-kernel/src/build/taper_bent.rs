use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use crate::{
    bspline::BSpline,
    build::SweepError,
    curve::BSplineCurve,
    error::GeometryError,
    profile::{Bend, graded},
    surface::{BSplineSurface, Surface},
};

const CUBIC: usize = 3;
const RULED_KNOTS: [f64; 4] = [0.0, 0.0, 1.0, 1.0];
const FIRST_SPANS: usize = 24;
const ROW_TOLERANCE: f64 = 1e-7;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Rung {
    pub height: f64,
    pub inset: f64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Rise<'a> {
    pub plane: &'a Plane,
    pub direction: Vector3,
}

impl Rise<'_> {
    pub(super) fn point(&self, point: Point2, height: f64) -> Point3 {
        self.plane.to_world(point) + self.direction * height
    }

    fn lifted(&self, bend: &Bend, fraction: f64, rung: Rung) -> Point3 {
        self.point(bend.offset_point(fraction, rung.inset), rung.height)
    }

    fn row(
        &self,
        bend: &Bend,
        rung: Rung,
        fractions: &[f64],
    ) -> Result<BSplineCurve, GeometryError> {
        let points: Vec<Point3> = fractions
            .iter()
            .map(|fraction| self.lifted(bend, *fraction, rung))
            .collect();
        BSpline::interpolating_at(CUBIC, fractions, &points)
    }

    fn strays(&self, bend: &Bend, rung: Rung, row: &BSplineCurve, nodes: &[f64]) -> Vec<f64> {
        nodes
            .windows(2)
            .map(|pair| {
                let fraction = pair.iter().sum::<f64>() * 0.5;
                row.point(fraction)
                    .distance(self.lifted(bend, fraction, rung))
            })
            .collect()
    }
}

pub(super) fn offset_surface(
    rise: Rise<'_>,
    bend: &Bend,
    rungs: [Rung; 2],
    overrun: [f64; 2],
) -> Result<Surface, SweepError> {
    let [low, high] = rungs;
    let rows = |nodes: &[f64]| -> Result<(BSplineCurve, BSplineCurve), GeometryError> {
        Ok((rise.row(bend, low, nodes)?, rise.row(bend, high, nodes)?))
    };
    let deviations = |nodes: &[f64]| -> Option<Vec<f64>> {
        let (bottom, top) = rows(nodes).ok()?;
        Some(
            rise.strays(bend, low, &bottom, nodes)
                .into_iter()
                .zip(rise.strays(bend, high, &top, nodes))
                .map(|(near, far)| near.max(far))
                .collect(),
        )
    };
    let nodes = graded(
        (-overrun[0], 1.0 + overrun[1]),
        &[0.0, 1.0],
        FIRST_SPANS,
        deviations,
        ROW_TOLERANCE,
    );
    let (bottom, top) = rows(&nodes)?;
    Ok(ruled(&bottom, &top)?.into())
}

fn ruled(bottom: &BSplineCurve, top: &BSplineCurve) -> Result<BSplineSurface, GeometryError> {
    let columns = bottom.control_points().len();
    let points: Vec<Point3> = bottom
        .control_points()
        .iter()
        .chain(top.control_points())
        .copied()
        .collect();
    BSplineSurface::new(
        CUBIC,
        1,
        bottom.knots().to_vec(),
        RULED_KNOTS.to_vec(),
        columns,
        points,
        None,
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Ring {
    pub height: f64,
    pub radius: f64,
}

const QUARTERS: usize = 4;

pub(super) fn leaning_cone(
    rise: Rise<'_>,
    center: Point2,
    seam: f64,
    rings: [Ring; 2],
) -> Result<Surface, SweepError> {
    if rings
        .iter()
        .any(|ring| !ring.radius.is_finite() || ring.radius <= 0.0)
    {
        return Err(SweepError::Unassembled);
    }
    let middle_weight = FRAC_PI_4.cos();
    let mut offsets = Vec::with_capacity(2 * QUARTERS + 1);
    let mut weights = Vec::with_capacity(2 * QUARTERS + 1);
    for quarter in 0..QUARTERS {
        let angle = seam + FRAC_PI_2 * quarter as f64;
        offsets.push(Point2::from_angle(angle));
        weights.push(1.0);
        offsets.push(Point2::from_angle(angle + FRAC_PI_4) / middle_weight);
        weights.push(middle_weight);
    }
    offsets.push(Point2::from_angle(seam));
    weights.push(1.0);
    let mut knots = vec![0.0; 3];
    for quarter in 1..QUARTERS {
        let knot = quarter as f64 / QUARTERS as f64;
        knots.extend([knot, knot]);
    }
    knots.extend([1.0; 3]);
    let columns = offsets.len();
    let points: Vec<Point3> = rings
        .iter()
        .flat_map(|ring| {
            offsets
                .iter()
                .map(move |offset| rise.point(center + *offset * ring.radius, ring.height))
        })
        .collect();
    let weights: Vec<f64> = weights.iter().chain(&weights).copied().collect();
    Ok(BSplineSurface::new(
        2,
        1,
        knots,
        RULED_KNOTS.to_vec(),
        columns,
        points,
        Some(weights),
    )?
    .into())
}
