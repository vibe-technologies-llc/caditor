use caditor_geometry::{Plane, Point3};

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

fn lifted(plane: &Plane, bend: &Bend, fraction: f64, rung: Rung) -> Point3 {
    plane.to_world(bend.offset_point(fraction, rung.inset)) + plane.normal() * rung.height
}

fn row(
    plane: &Plane,
    bend: &Bend,
    rung: Rung,
    fractions: &[f64],
) -> Result<BSplineCurve, GeometryError> {
    let points: Vec<Point3> = fractions
        .iter()
        .map(|fraction| lifted(plane, bend, *fraction, rung))
        .collect();
    BSpline::interpolating_at(CUBIC, fractions, &points)
}

fn strays(plane: &Plane, bend: &Bend, rung: Rung, row: &BSplineCurve, nodes: &[f64]) -> Vec<f64> {
    nodes
        .windows(2)
        .map(|pair| {
            let fraction = pair.iter().sum::<f64>() * 0.5;
            row.point(fraction)
                .distance(lifted(plane, bend, fraction, rung))
        })
        .collect()
}

pub(super) fn offset_surface(
    plane: &Plane,
    bend: &Bend,
    rungs: [Rung; 2],
    overrun: [f64; 2],
) -> Result<Surface, SweepError> {
    let [low, high] = rungs;
    let rows = |nodes: &[f64]| -> Result<(BSplineCurve, BSplineCurve), GeometryError> {
        Ok((
            row(plane, bend, low, nodes)?,
            row(plane, bend, high, nodes)?,
        ))
    };
    let deviations = |nodes: &[f64]| -> Option<Vec<f64>> {
        let (bottom, top) = rows(nodes).ok()?;
        Some(
            strays(plane, bend, low, &bottom, nodes)
                .into_iter()
                .zip(strays(plane, bend, high, &top, nodes))
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
