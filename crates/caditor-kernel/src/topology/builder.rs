use std::collections::BTreeMap;

use caditor_geometry::{Point2, Point3, Vector2};
use thiserror::Error;

use crate::{
    curve::{Curve, Line},
    error::GeometryError,
    interrupt::Interrupted,
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin},
    sense::Sense,
    surface::Surface,
    tessellation::TessellationError,
    tolerance::LINEAR_RESOLUTION,
    topology::{
        Coedge, CoedgeId, Edge, EdgeId, Face, FaceId, Loop, LoopId, Shell, ShellId, Solid, Vertex,
        VertexId,
        pcurve::{Pcurve, PcurveError, fit},
        validate::ValidationError,
    },
};

const INTERVAL_SLACK: f64 = 1e-9;
const LOOP_PLACEMENT_SLACK: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum BuildError {
    #[error("vertex {0:?} does not exist")]
    UnknownVertex(VertexId),
    #[error("edge {0:?} does not exist")]
    UnknownEdge(EdgeId),
    #[error("face {0:?} does not exist")]
    UnknownFace(FaceId),
    #[error("shell {0:?} does not exist")]
    UnknownShell(ShellId),
    #[error("a vertex point is not finite")]
    NonFinitePoint,
    #[error("vertex {vertex:?} lies {distance} from the end of its edge")]
    VertexOffCurve { vertex: VertexId, distance: f64 },
    #[error("the edge interval lies outside the domain of its curve")]
    IntervalOutsideDomain,
    #[error("the edge has no length")]
    ZeroLengthEdge,
    #[error("a loop needs at least one coedge")]
    EmptyLoop,
    #[error("the solid has too many entities")]
    TooManyEntities,
    #[error("edge {edge:?} has no pcurve on its face: {error}")]
    Pcurve { edge: EdgeId, error: PcurveError },
    #[error(transparent)]
    Geometry(#[from] GeometryError),
    #[error("the solid is invalid: {0}")]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl BuildError {
    pub fn interrupted(&self) -> Option<Interrupted> {
        match self {
            Self::Invalid(
                ValidationError::Tessellation(TessellationError::Cancelled(interrupted))
                | ValidationError::Cancelled(interrupted),
            )
            | Self::Cancelled(interrupted) => Some(*interrupted),
            _ => None,
        }
    }

    pub(crate) fn pcurve(edge: EdgeId, error: PcurveError) -> Self {
        match error {
            PcurveError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Pcurve { edge, error },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SolidBuilder {
    solid: Solid,
}

impl SolidBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn vertex(&mut self, point: Point3) -> Result<VertexId, BuildError> {
        if !point.is_finite() {
            return Err(BuildError::NonFinitePoint);
        }
        let id =
            VertexId::from_index(self.solid.vertices.len()).ok_or(BuildError::TooManyEntities)?;
        self.solid.vertices.push(Vertex { point });
        Ok(id)
    }

    pub fn edge(
        &mut self,
        curve: Curve,
        interval: Interval,
        start: VertexId,
        end: VertexId,
    ) -> Result<EdgeId, BuildError> {
        let start_point = self.point_of(start)?;
        let end_point = self.point_of(end)?;
        check_interval(&curve, interval)?;
        for (vertex, point, parameter) in [
            (start, start_point, interval.start()),
            (end, end_point, interval.end()),
        ] {
            let distance = curve.point(parameter).distance(point);
            if distance.is_nan() || distance > LINEAR_RESOLUTION {
                return Err(BuildError::VertexOffCurve { vertex, distance });
            }
        }
        let length = curve.length(interval);
        if length.is_nan() || length <= LINEAR_RESOLUTION {
            return Err(BuildError::ZeroLengthEdge);
        }
        let id = EdgeId::from_index(self.solid.edges.len()).ok_or(BuildError::TooManyEntities)?;
        self.solid.edges.push(Edge {
            curve,
            interval,
            start,
            end,
            name: EdgeName::NONE,
            coedges: Vec::new(),
        });
        Ok(id)
    }

    pub fn line_edge(&mut self, start: VertexId, end: VertexId) -> Result<EdgeId, BuildError> {
        let (from, to) = (self.point_of(start)?, self.point_of(end)?);
        let line = Line::through(from, to)?;
        let interval = Interval::new(0.0, from.distance(to)).ok_or(BuildError::NonFinitePoint)?;
        self.edge(line.into(), interval, start, end)
    }

    pub fn set_edge_name(&mut self, edge: EdgeId, name: EdgeName) -> Result<(), BuildError> {
        self.solid
            .edges
            .get_mut(edge.index())
            .ok_or(BuildError::UnknownEdge(edge))?
            .name = name;
        Ok(())
    }

    pub fn shell(&mut self) -> Result<ShellId, BuildError> {
        let id = ShellId::from_index(self.solid.shells.len()).ok_or(BuildError::TooManyEntities)?;
        self.solid.shells.push(Shell::default());
        Ok(id)
    }

    pub fn face(
        &mut self,
        shell: ShellId,
        surface: Surface,
        sense: Sense,
    ) -> Result<FaceId, BuildError> {
        let id = FaceId::from_index(self.solid.faces.len()).ok_or(BuildError::TooManyEntities)?;
        self.solid
            .shells
            .get_mut(shell.index())
            .ok_or(BuildError::UnknownShell(shell))?
            .faces
            .push(id);
        self.solid.faces.push(Face {
            surface,
            sense,
            loops: Vec::new(),
            shell,
            name: FaceName::NONE,
            origin: None,
        });
        Ok(id)
    }

    pub fn set_face_name(&mut self, face: FaceId, name: FaceName) -> Result<(), BuildError> {
        self.solid
            .faces
            .get_mut(face.index())
            .ok_or(BuildError::UnknownFace(face))?
            .name = name;
        Ok(())
    }

    pub fn set_face_origin(&mut self, face: FaceId, origin: FaceOrigin) -> Result<(), BuildError> {
        self.solid
            .faces
            .get_mut(face.index())
            .ok_or(BuildError::UnknownFace(face))?
            .origin = Some(origin);
        Ok(())
    }

    pub fn add_loop(
        &mut self,
        face: FaceId,
        coedges: &[(EdgeId, Sense)],
    ) -> Result<LoopId, BuildError> {
        let surface = self
            .solid
            .face(face)
            .ok_or(BuildError::UnknownFace(face))?
            .surface
            .clone();
        let mut hint: Option<Point2> = None;
        let mut fitted = Vec::with_capacity(coedges.len());
        for &(edge_id, sense) in coedges {
            let edge = self
                .solid
                .edge(edge_id)
                .ok_or(BuildError::UnknownEdge(edge_id))?;
            let pcurve = fit(&surface, &edge.curve, edge.interval, sense, hint)
                .map_err(|error| BuildError::pcurve(edge_id, error))?;
            hint = Some(pcurve.end());
            fitted.push((edge_id, sense, pcurve));
        }
        let placed = place_loop(&surface, self.first_loop_bounds(face), fitted);
        let id = self.add_loop_with_pcurves(face, placed)?;
        self.align_seams(face);
        Ok(id)
    }

    fn first_loop_bounds(&self, face: FaceId) -> Option<(Point2, Point2)> {
        let first = *self.solid.face(face)?.loops.first()?;
        let face_loop = self.solid.face_loop(first)?;
        let mut samples = face_loop
            .coedges
            .iter()
            .filter_map(|coedge| self.solid.coedge(*coedge))
            .flat_map(|coedge| coedge.pcurve.samples().iter().map(|sample| sample.uv));
        let start = samples.next()?;
        Some(samples.fold((start, start), |(low, high), uv| {
            (low.min(uv), high.max(uv))
        }))
    }

    pub fn add_loop_with_pcurves(
        &mut self,
        face: FaceId,
        coedges: Vec<(EdgeId, Sense, Pcurve)>,
    ) -> Result<LoopId, BuildError> {
        if coedges.is_empty() {
            return Err(BuildError::EmptyLoop);
        }
        if self.solid.face(face).is_none() {
            return Err(BuildError::UnknownFace(face));
        }
        if let Some((edge, _, _)) = coedges
            .iter()
            .find(|(edge, _, _)| self.solid.edge(*edge).is_none())
        {
            return Err(BuildError::UnknownEdge(*edge));
        }
        let loop_id =
            LoopId::from_index(self.solid.loops.len()).ok_or(BuildError::TooManyEntities)?;
        let mut members = Vec::with_capacity(coedges.len());
        for (edge, sense, pcurve) in coedges {
            let id = CoedgeId::from_index(self.solid.coedges.len())
                .ok_or(BuildError::TooManyEntities)?;
            if let Some(edge) = self.solid.edges.get_mut(edge.index()) {
                edge.coedges.push(id);
            }
            self.solid.coedges.push(Coedge {
                edge,
                sense,
                owner: loop_id,
                pcurve,
            });
            members.push(id);
        }
        self.solid.loops.push(Loop {
            face,
            coedges: members,
        });
        if let Some(face) = self.solid.faces.get_mut(face.index()) {
            face.loops.push(loop_id);
        }
        Ok(loop_id)
    }

    pub fn build(self) -> Result<Solid, BuildError> {
        self.solid.validate()?;
        Ok(self.solid)
    }

    #[cfg(test)]
    pub(crate) fn build_unchecked(self) -> Solid {
        self.solid
    }

    pub fn vertex_point(&self, vertex: VertexId) -> Option<Point3> {
        self.solid.vertex(vertex).map(Vertex::point)
    }

    fn point_of(&self, vertex: VertexId) -> Result<Point3, BuildError> {
        self.solid
            .vertex(vertex)
            .map(Vertex::point)
            .ok_or(BuildError::UnknownVertex(vertex))
    }

    fn align_seams(&mut self, face: FaceId) {
        let Some(face) = self.solid.face(face) else {
            return;
        };
        let surface = face.surface.clone();
        let face_sense = face.sense;
        let mut uses: BTreeMap<EdgeId, Vec<CoedgeId>> = BTreeMap::new();
        for coedge in face
            .loops
            .iter()
            .filter_map(|id| self.solid.face_loop(*id))
            .flat_map(|face_loop| face_loop.coedges.iter().copied())
        {
            if let Some(data) = self.solid.coedge(coedge) {
                uses.entry(data.edge).or_default().push(coedge);
            }
        }
        for pair in uses.into_values() {
            let [first, second] = pair.as_slice() else {
                continue;
            };
            let (Some(first_data), Some(second_data)) =
                (self.solid.coedge(*first), self.solid.coedge(*second))
            else {
                continue;
            };
            let Some(offset) = seam_offset(
                &surface,
                face_sense,
                &first_data.pcurve,
                &second_data.pcurve,
            ) else {
                continue;
            };
            if let Some(data) = self.solid.coedges.get_mut(second.index()) {
                data.pcurve = data.pcurve.shifted(offset);
            }
        }
    }
}

fn check_interval(curve: &Curve, interval: Interval) -> Result<(), BuildError> {
    let slack = INTERVAL_SLACK * (1.0 + interval.start().abs().max(interval.end().abs()));
    let inside = match curve.period() {
        Some(period) => interval.length() <= period + slack,
        None => {
            let domain = curve.domain();
            interval.start() >= domain.start() - slack && interval.end() <= domain.end() + slack
        }
    };
    if inside {
        Ok(())
    } else {
        Err(BuildError::IntervalOutsideDomain)
    }
}

fn place_loop(
    surface: &Surface,
    outer: Option<(Point2, Point2)>,
    fitted: Vec<(EdgeId, Sense, Pcurve)>,
) -> Vec<(EdgeId, Sense, Pcurve)> {
    let uvs: Vec<Point2> = fitted
        .iter()
        .flat_map(|(_, _, pcurve)| pcurve.samples().iter().map(|sample| sample.uv))
        .collect();
    let Some(first) = uvs.first() else {
        return fitted;
    };
    let (low, high) = uvs.iter().fold((*first, *first), |(low, high), uv| {
        (low.min(*uv), high.max(*uv))
    });
    let middle = (low + high) * 0.5;
    let shift = |period: Option<f64>, value: f64, target_low: f64| -> f64 {
        match period.filter(|period| *period > 0.0 && period.is_finite()) {
            Some(period) => -((value - target_low) / period).floor() * period,
            None => 0.0,
        }
    };
    let offset = match outer {
        None => Vector2::new(
            shift(surface.u_period(), low.x + LOOP_PLACEMENT_SLACK, 0.0),
            shift(surface.v_period(), low.y + LOOP_PLACEMENT_SLACK, 0.0),
        ),
        Some((outer_low, _)) => Vector2::new(
            shift(surface.u_period(), middle.x, outer_low.x),
            shift(surface.v_period(), middle.y, outer_low.y),
        ),
    };
    if offset == Vector2::ZERO {
        return fitted;
    }
    fitted
        .into_iter()
        .map(|(edge, sense, pcurve)| (edge, sense, pcurve.shifted(offset)))
        .collect()
}

fn mean_uv(pcurve: &Pcurve) -> Point2 {
    let samples = pcurve.samples();
    let total = samples
        .iter()
        .fold(Point2::ZERO, |sum, sample| sum + sample.uv);
    total / samples.len().max(1) as f64
}

fn seam_offset(
    surface: &Surface,
    face_sense: Sense,
    first: &Pcurve,
    second: &Pcurve,
) -> Option<Vector2> {
    let travel = first.end() - first.start();
    let apart = mean_uv(second) - mean_uv(first);
    if let Some(period) = surface.u_period()
        && travel.y.abs() >= travel.x.abs()
    {
        let first_on_larger_u = (travel.y > 0.0) == face_sense.is_same();
        let shift = if first_on_larger_u { -period } else { period };
        return (apart.x.abs() < 0.5 * period).then_some(Vector2::new(shift, 0.0));
    }
    if let Some(period) = surface.v_period()
        && travel.x.abs() > travel.y.abs()
    {
        let first_on_smaller_v = (travel.x > 0.0) == face_sense.is_same();
        let shift = if first_on_smaller_v { period } else { -period };
        return (apart.y.abs() < 0.5 * period).then_some(Vector2::new(0.0, shift));
    }
    None
}
