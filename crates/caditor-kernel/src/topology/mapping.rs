use caditor_geometry::{Point3, Similarity};
use thiserror::Error;

use crate::{
    curve::{Curve, IntersectionCurve},
    error::GeometryError,
    interrupt::{self, Interrupted},
    interval::Interval,
    mapping::{Affine, UvMap},
    tessellation::TessellationError,
    tolerance::{INTERSECTION_TOLERANCE, LINEAR_RESOLUTION, MAX_SIZE},
    topology::{
        CoedgeId, EdgeId, Solid, Vertex, VertexId,
        pcurve::{Pcurve, PcurveError, PcurveSample, fit, settle_pole_ends},
        validate::ValidationError,
    },
};

const GAP_SAMPLES: usize = 32;
const MIN_RETRACE_SAMPLES: usize = 16;
const MAX_RETRACE_SAMPLES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum TransformError {
    #[error(transparent)]
    Geometry(#[from] GeometryError),
    #[error("edge {edge:?} left its faces once scaled and could not be traced on them again")]
    EdgeOffFaces { edge: EdgeId },
    #[error("edge {edge:?} has no pcurve on its face once transformed: {error}")]
    Pcurve { edge: EdgeId, error: PcurveError },
    #[error("the transformed solid is invalid: {0}")]
    Invalid(ValidationError),
    #[error(transparent)]
    Cancelled(#[from] Interrupted),
}

impl TransformError {
    fn pcurve(edge: EdgeId, error: PcurveError) -> Self {
        match error {
            PcurveError::Cancelled(interrupted) => Self::Cancelled(interrupted),
            error => Self::Pcurve { edge, error },
        }
    }

    fn invalid(error: ValidationError) -> Self {
        match error {
            ValidationError::Cancelled(interrupted)
            | ValidationError::Tessellation(TessellationError::Cancelled(interrupted)) => {
                Self::Cancelled(interrupted)
            }
            error => Self::Invalid(error),
        }
    }
}

impl Solid {
    pub fn mapped(&self, similarity: &Similarity) -> Result<Self, TransformError> {
        let mut mapped = self.mapped_exactly(similarity)?;
        if similarity.scale() > 1.0 {
            mapped.settle_enlarged_edges()?;
        }
        if !similarity.is_rigid() {
            mapped.validate().map_err(TransformError::invalid)?;
        }
        Ok(mapped)
    }

    fn mapped_exactly(&self, similarity: &Similarity) -> Result<Self, TransformError> {
        let mirrored = similarity.is_mirrored();
        let mut mapped = self.clone();
        for vertex in &mut mapped.vertices {
            let point = similarity.apply_point(vertex.point);
            if !point.is_finite() {
                return Err(GeometryError::NonFinite.into());
            }
            let reach = point.abs().max_element();
            if reach > MAX_SIZE {
                return Err(GeometryError::BeyondMaximum(reach).into());
            }
            *vertex = Vertex { point };
        }
        let mut edge_maps: Vec<Affine> = Vec::with_capacity(self.edges.len());
        for edge in &mut mapped.edges {
            let (curve, along) = edge.curve.mapped(similarity)?;
            edge.interval = along
                .interval(edge.interval)
                .ok_or(GeometryError::NonFinite)?;
            edge.curve = curve;
            if similarity.scale() < 1.0 {
                let length = edge.curve.length(edge.interval);
                if length < LINEAR_RESOLUTION {
                    return Err(GeometryError::BelowResolution(length).into());
                }
            }
            edge_maps.push(along);
        }
        let mut face_maps: Vec<UvMap> = Vec::with_capacity(self.faces.len());
        for face in &mut mapped.faces {
            let (surface, map) = face.surface.mapped(similarity)?;
            face.surface = surface;
            if mirrored == map.keeps_orientation() {
                face.sense = face.sense.reversed();
            }
            face_maps.push(map);
        }
        for face_loop in &mut mapped.loops {
            let Some(face_map) = face_maps.get(face_loop.face.index()) else {
                continue;
            };
            for id in &face_loop.coedges {
                let Some(coedge) = mapped.coedges.get_mut(id.index()) else {
                    continue;
                };
                let Some(edge_map) = edge_maps.get(coedge.edge.index()) else {
                    continue;
                };
                let Some(face) = mapped.faces.get(face_loop.face.index()) else {
                    continue;
                };
                let mut samples: Vec<PcurveSample> = coedge
                    .pcurve
                    .samples()
                    .iter()
                    .map(|sample| PcurveSample {
                        parameter: edge_map.apply(sample.parameter),
                        uv: face_map.apply(sample.uv),
                    })
                    .collect();
                if mirrored {
                    samples.reverse();
                    coedge.sense = coedge.sense.reversed();
                }
                settle_pole_ends(&face.surface, &mut samples);
                coedge.pcurve = Pcurve::new(samples, coedge.pcurve.tolerance())
                    .map_err(|error| TransformError::pcurve(coedge.edge, error))?;
            }
            if mirrored {
                face_loop.coedges.reverse();
            }
        }
        Ok(mapped)
    }

    fn settle_enlarged_edges(&mut self) -> Result<(), TransformError> {
        for index in 0..self.edges.len() {
            interrupt::check()?;
            let Some(id) = EdgeId::from_index(index) else {
                continue;
            };
            if self.edge_leaves_its_faces(id) {
                self.retrace(id)?;
            } else {
                self.refine_pcurves(id)?;
            }
        }
        Ok(())
    }

    fn edge_leaves_its_faces(&self, id: EdgeId) -> bool {
        let Some(edge) = self.edge(id) else {
            return false;
        };
        if self.distinct_faces(id).is_none() {
            return false;
        }
        if matches!(edge.curve, Curve::Intersection(_)) {
            return true;
        }
        edge.coedges.iter().any(|coedge_id| {
            let (Some(coedge), Some(face)) = (
                self.coedge(*coedge_id),
                self.coedge_face(*coedge_id)
                    .and_then(|face| self.face(face)),
            ) else {
                return false;
            };
            edge.interval.split(GAP_SAMPLES).any(|parameter| {
                let point = edge.curve.point(parameter);
                let uv = face
                    .surface
                    .project(point, Some(coedge.pcurve.uv_at(parameter)));
                face.surface.point_at(uv).distance(point) > INTERSECTION_TOLERANCE
            })
        })
    }

    fn distinct_faces(&self, id: EdgeId) -> Option<[CoedgeId; 2]> {
        let [first, second] = *self.edge(id)?.coedges.as_slice() else {
            return None;
        };
        (self.coedge_face(first)? != self.coedge_face(second)?).then_some([first, second])
    }

    fn retrace(&mut self, id: EdgeId) -> Result<(), TransformError> {
        let off_faces = TransformError::EdgeOffFaces { edge: id };
        let [first, second] = self.distinct_faces(id).ok_or(off_faces.clone())?;
        let surface_of = |coedge: CoedgeId| {
            self.coedge_face(coedge)
                .and_then(|face| self.face(face))
                .map(|face| face.surface.clone())
                .ok_or(off_faces.clone())
        };
        let surfaces = [surface_of(first)?, surface_of(second)?];
        let edge = self.edge(id).ok_or(off_faces.clone())?;
        let (from, to) = (self.vertex_point(edge.start)?, self.vertex_point(edge.end)?);
        let count = match &edge.curve {
            Curve::Intersection(curve) => curve.nodes().len(),
            _ => GAP_SAMPLES,
        }
        .clamp(MIN_RETRACE_SAMPLES, MAX_RETRACE_SAMPLES);
        let mut points: Vec<Point3> = edge
            .interval
            .split(count)
            .map(|parameter| edge.curve.point(parameter))
            .collect();
        if let Some(point) = points.first_mut() {
            *point = from;
        }
        if let Some(point) = points.last_mut() {
            *point = to;
        }
        let closed = edge.is_closed();
        let traced =
            IntersectionCurve::through(surfaces, &points, closed).ok_or(off_faces.clone())?;
        let interval = traced.domain();
        let fits = traced.point(interval.start()).distance(from) <= LINEAR_RESOLUTION
            && traced.point(interval.end()).distance(to) <= LINEAR_RESOLUTION;
        if !fits {
            return Err(off_faces);
        }
        let curve = Curve::Intersection(traced);
        for coedge_id in [first, second] {
            self.refit_pcurve(coedge_id, &curve, interval)?;
        }
        if let Some(edge) = self.edges.get_mut(id.index()) {
            edge.curve = curve;
            edge.interval = interval;
        }
        Ok(())
    }

    fn vertex_point(&self, id: VertexId) -> Result<Point3, TransformError> {
        self.vertex(id)
            .map(Vertex::point)
            .ok_or(TransformError::Invalid(ValidationError::MissingEntity))
    }

    fn refit_pcurve(
        &mut self,
        id: CoedgeId,
        curve: &Curve,
        interval: Interval,
    ) -> Result<(), TransformError> {
        let Some(coedge) = self.coedge(id) else {
            return Ok(());
        };
        let Some(face) = self.coedge_face(id).and_then(|face| self.face(face)) else {
            return Ok(());
        };
        let refitted = fit(
            &face.surface,
            curve,
            interval,
            coedge.sense,
            Some(coedge.pcurve.start()),
        )
        .map_err(|error| TransformError::pcurve(coedge.edge, error))?;
        if let Some(coedge) = self.coedges.get_mut(id.index()) {
            coedge.pcurve = refitted;
        }
        Ok(())
    }

    fn refine_pcurves(&mut self, id: EdgeId) -> Result<(), TransformError> {
        let Some(edge) = self.edge(id) else {
            return Ok(());
        };
        let mut refined = Vec::with_capacity(edge.coedges.len());
        for coedge_id in &edge.coedges {
            let (Some(coedge), Some(face)) = (
                self.coedge(*coedge_id),
                self.coedge_face(*coedge_id)
                    .and_then(|face| self.face(face)),
            ) else {
                continue;
            };
            let pcurve = coedge
                .pcurve
                .refined(&face.surface, &edge.curve)
                .map_err(|error| TransformError::pcurve(id, error))?;
            refined.push((*coedge_id, pcurve));
        }
        for (coedge_id, pcurve) in refined {
            if let Some(coedge) = self.coedges.get_mut(coedge_id.index()) {
                coedge.pcurve = pcurve;
            }
        }
        Ok(())
    }
}
