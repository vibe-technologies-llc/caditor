use caditor_geometry::{Plane, Point3, RigidTransform, Rotation3, Vector3};

use super::{PathSweepError, WindingAxis};
use crate::{
    bspline::BSpline,
    curve::{BSplineCurve, Curve},
    error::GeometryError,
    interrupt,
    interval::Interval,
    surface::BSplineSurface,
};

pub(super) const FIT_TOLERANCE: f64 = 1e-5;
const MAX_STATIONS: usize = 2048;
const STATION_DEGREE: usize = 3;

pub(super) type FramesAt<'a> = dyn Fn(&[f64]) -> Result<Vec<RigidTransform>, PathSweepError> + 'a;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Stations {
    parameters: Vec<f64>,
    frames: Vec<RigidTransform>,
}

fn uniform(range: Interval, intervals: usize) -> Vec<f64> {
    (0..=intervals)
        .map(|index| range.at(index as f64 / intervals as f64))
        .collect()
}

fn interpolated(parameters: &[f64], points: &[Point3]) -> Result<BSplineCurve, GeometryError> {
    BSpline::interpolating_at(STATION_DEGREE, parameters, points)
}

impl Stations {
    pub(super) fn fit(
        range: Interval,
        initial: usize,
        frames_at: &FramesAt<'_>,
        points: &[Point3],
        segment: usize,
    ) -> Result<Self, PathSweepError> {
        let mut intervals = initial.max(STATION_DEGREE + 1);
        loop {
            interrupt::check()?;
            if intervals > MAX_STATIONS {
                return Err(PathSweepError::TooIntricate { segment });
            }
            let parameters = uniform(range, intervals);
            let frames = frames_at(&parameters)?;
            let finer = uniform(range, 2 * intervals);
            let finer_frames = frames_at(&finer)?;
            let stations = Self { parameters, frames };
            if stations.deviation(points, &finer, &finer_frames)? <= FIT_TOLERANCE {
                return Ok(stations);
            }
            intervals *= 2;
        }
    }

    fn deviation(
        &self,
        points: &[Point3],
        finer: &[f64],
        finer_frames: &[RigidTransform],
    ) -> Result<f64, PathSweepError> {
        let mut worst: f64 = 0.0;
        for point in points {
            interrupt::check()?;
            let trajectory = self.trajectory(*point)?;
            for (parameter, frame) in finer.iter().zip(finer_frames).skip(1).step_by(2) {
                let gap = trajectory
                    .point(*parameter)
                    .distance(frame.apply_point(*point));
                if !gap.is_finite() {
                    return Err(GeometryError::NonFinite.into());
                }
                worst = worst.max(gap);
            }
        }
        Ok(worst)
    }

    pub(super) fn first(&self) -> Option<RigidTransform> {
        self.frames.first().copied()
    }

    pub(super) fn last(&self) -> Option<RigidTransform> {
        self.frames.last().copied()
    }

    pub(super) fn middle(&self) -> Option<RigidTransform> {
        self.frames.get(self.frames.len() / 2).copied()
    }

    pub(super) fn parameters(&self) -> &[f64] {
        &self.parameters
    }

    pub(super) fn frames(&self) -> &[RigidTransform] {
        &self.frames
    }

    pub(super) fn closing_on(&mut self, frame: RigidTransform) {
        if let Some(last) = self.frames.last_mut() {
            *last = frame;
        }
    }

    pub(super) fn trajectory(&self, point: Point3) -> Result<BSplineCurve, GeometryError> {
        let rows: Vec<Point3> = self
            .frames
            .iter()
            .map(|frame| frame.apply_point(point))
            .collect();
        interpolated(&self.parameters, &rows)
    }

    pub(super) fn surface(&self, profile: &BSplineCurve) -> Result<BSplineSurface, GeometryError> {
        let columns = profile
            .control_points()
            .iter()
            .map(|point| self.trajectory(*point))
            .collect::<Result<Vec<_>, _>>()?;
        let first = columns.first().ok_or(GeometryError::DegenerateSurface)?;
        let rows = first.control_points().len();
        let width = columns.len();
        let mut net = Vec::with_capacity(rows * width);
        for row in 0..rows {
            for column in &columns {
                net.push(
                    column
                        .control_points()
                        .get(row)
                        .copied()
                        .ok_or(GeometryError::DegenerateSurface)?,
                );
            }
        }
        let weights = profile.weights().map(|weights| {
            (0..rows)
                .flat_map(|_| weights.iter().copied())
                .collect::<Vec<f64>>()
        });
        BSplineSurface::new(
            profile.degree(),
            first.degree(),
            profile.knots().to_vec(),
            first.knots().to_vec(),
            width,
            net,
            weights,
        )
    }
}

pub(super) fn sliding_surface(
    profile: &BSplineCurve,
    path: &BSplineCurve,
    from: Point3,
) -> Result<BSplineSurface, GeometryError> {
    let width = profile.control_points().len();
    let mut net = Vec::with_capacity(width * path.control_points().len());
    for along in path.control_points() {
        for point in profile.control_points() {
            net.push(*point + (*along - from));
        }
    }
    let weights = match (profile.weights(), path.weights()) {
        (None, None) => None,
        (across, along) => {
            let across: Vec<f64> = across.map_or_else(|| vec![1.0; width], <[f64]>::to_vec);
            let along: Vec<f64> =
                along.map_or_else(|| vec![1.0; path.control_points().len()], <[f64]>::to_vec);
            Some(
                along
                    .iter()
                    .flat_map(|row| across.iter().map(move |column| row * column))
                    .collect(),
            )
        }
    };
    BSplineSurface::new(
        profile.degree(),
        path.degree(),
        profile.knots().to_vec(),
        path.knots().to_vec(),
        width,
        net,
        weights,
    )
}

fn any_perpendicular(direction: Vector3) -> Vector3 {
    let reference = if direction.x.abs() < 0.9 {
        Vector3::X
    } else {
        Vector3::Y
    };
    direction.cross(reference).normalize_or_zero()
}

fn basis(tangent: Vector3, across: Vector3) -> Option<Rotation3> {
    let frame = Plane::from_frame(Point3::ZERO, tangent.cross(across), tangent)?;
    RigidTransform::from_frame(&frame).map(|placed| placed.rotation())
}

pub(super) fn rotation_minimising(
    path: &Curve,
    parameters: &[f64],
    start: RigidTransform,
) -> Result<Vec<RigidTransform>, PathSweepError> {
    let degenerate = || PathSweepError::from(GeometryError::ZeroDirection);
    let Some(first) = parameters.first() else {
        return Ok(Vec::new());
    };
    let origin = path.evaluate(*first);
    let mut tangent = origin.first.try_normalize().ok_or_else(degenerate)?;
    let mut across = any_perpendicular(tangent);
    let initial = basis(tangent, across).ok_or_else(degenerate)?;
    let mut point = origin.point;
    let mut frames = Vec::with_capacity(parameters.len());
    frames.push(start);
    for parameter in parameters.iter().skip(1) {
        interrupt::check()?;
        let next = path.evaluate(*parameter);
        let next_tangent = next.first.try_normalize().ok_or_else(degenerate)?;
        let step = next.point - point;
        let reach = step.length_squared();
        let (reflected_across, reflected_tangent) = if reach > 0.0 {
            (
                across - step * (2.0 * step.dot(across) / reach),
                tangent - step * (2.0 * step.dot(tangent) / reach),
            )
        } else {
            (across, tangent)
        };
        let second = next_tangent - reflected_tangent;
        let second_reach = second.length_squared();
        let turned = if second_reach > 0.0 {
            reflected_across - second * (2.0 * second.dot(reflected_across) / second_reach)
        } else {
            reflected_across
        };
        across = (turned - next_tangent * next_tangent.dot(turned))
            .try_normalize()
            .ok_or_else(degenerate)?;
        tangent = next_tangent;
        point = next.point;
        let rotation = basis(tangent, across).ok_or_else(degenerate)? * initial.conjugate();
        let motion = RigidTransform::new(rotation, point - rotation * origin.point)
            .ok_or_else(degenerate)?;
        frames.push(start.then(&motion));
    }
    Ok(frames)
}

pub(super) fn rotating(
    (origin, axis): (Point3, Vector3),
    from: f64,
    parameters: &[f64],
    start: RigidTransform,
) -> Result<Vec<RigidTransform>, PathSweepError> {
    parameters
        .iter()
        .map(|parameter| {
            RigidTransform::rotation_about(origin, axis, parameter - from)
                .map(|turn| start.then(&turn))
                .ok_or_else(|| GeometryError::ZeroDirection.into())
        })
        .collect()
}

pub(super) fn winding(
    path: &Curve,
    axis: WindingAxis,
    parameters: &[f64],
    start: RigidTransform,
) -> Result<Vec<RigidTransform>, PathSweepError> {
    let degenerate = || PathSweepError::from(GeometryError::ZeroDirection);
    let direction = axis.direction.try_normalize().ok_or_else(degenerate)?;
    let radial = |point: Point3| (point - axis.origin).reject_from(direction);
    let Some(first) = parameters.first() else {
        return Ok(Vec::new());
    };
    let origin = path.point(*first);
    let mut previous = radial(origin);
    let mut angle = 0.0;
    let mut frames = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        interrupt::check()?;
        let point = path.point(*parameter);
        let across = radial(point);
        angle += direction
            .dot(previous.cross(across))
            .atan2(previous.dot(across));
        previous = across;
        let turn =
            RigidTransform::rotation_about(axis.origin, direction, angle).ok_or_else(degenerate)?;
        let shift =
            RigidTransform::translation(point - turn.apply_point(origin)).ok_or_else(degenerate)?;
        frames.push(start.then(&turn.then(&shift)));
    }
    Ok(frames)
}
