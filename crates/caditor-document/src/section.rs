use caditor_geometry::{Plane, Point2, Vector2};
use caditor_kernel::{
    BooleanError, BooleanOperation, EdgeId, EdgeReference, Face, FaceName, FaceOrigin, Solid,
    boolean,
};

use crate::{
    document::FeatureId,
    projection::{Outline, ProjectionSource, edge_outline},
    split::{HalfSpaceError, half_space_solid},
    tolerance::DIRECTION_TOLERANCE,
};

#[derive(Debug, thiserror::Error)]
pub enum SectionError {
    #[error("the sketch plane does not cut through the body")]
    Misses,
    #[error("the cutting half-space could not be built: {0}")]
    HalfSpace(HalfSpaceError),
    #[error("the body could not be cut: {0}")]
    Boolean(BooleanError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionCurve {
    pub face: FaceName,
    pub source: ProjectionSource,
    pub outline: Outline,
}

pub(crate) fn section_solid(
    solid: &Solid,
    plane: &Plane,
    sketch: FeatureId,
) -> Result<Solid, SectionError> {
    let half =
        half_space_solid(solid, plane, false, sketch.raw()).map_err(|error| match error {
            HalfSpaceError::Misses => SectionError::Misses,
            other => SectionError::HalfSpace(other),
        })?;
    boolean(solid, &half, BooleanOperation::Intersection).map_err(|error| match error {
        BooleanError::Empty => SectionError::Misses,
        other => SectionError::Boolean(other),
    })
}

fn made_by_cut(origin: &FaceOrigin, sketch: FeatureId) -> bool {
    matches!(
        origin,
        FaceOrigin::StartCap { feature } | FaceOrigin::EndCap { feature }
            if *feature == sketch.raw()
    )
}

fn is_cut_face(face: &Face, sketch: FeatureId) -> bool {
    face.origin()
        .is_some_and(|origin| made_by_cut(&origin, sketch))
}

pub(crate) fn section_edges(section: &Solid, sketch: FeatureId) -> Vec<(EdgeId, FaceName)> {
    section
        .edges()
        .filter_map(|(id, edge)| {
            let faces: Vec<&Face> = edge
                .coedges()
                .iter()
                .filter_map(|coedge| section.coedge_face(*coedge))
                .filter_map(|face| section.face(face))
                .collect();
            let cut = faces.iter().any(|face| is_cut_face(face, sketch));
            let cut_from = faces.iter().find(|face| !is_cut_face(face, sketch))?;
            cut.then_some((id, cut_from.name()))
        })
        .collect()
}

pub(crate) fn section_reference(
    section: &Solid,
    edge: EdgeId,
    sketch: FeatureId,
) -> Option<EdgeReference> {
    let captured = EdgeReference::capture(section, edge)?;
    let origins = captured
        .origins()
        .map(|origin| origin.filter(|origin| !made_by_cut(origin, sketch)));
    Some(captured.with_origins(origins))
}

pub fn section_curves(
    body: FeatureId,
    solid: &Solid,
    plane: &Plane,
    sketch: FeatureId,
) -> Result<Vec<SectionCurve>, SectionError> {
    let section = section_solid(solid, plane, sketch)?;
    let curves: Vec<SectionCurve> = section_edges(&section, sketch)
        .into_iter()
        .filter_map(|(edge, face)| {
            let reference = section_reference(&section, edge, sketch)?;
            let outline = edge_outline(&section, edge, plane)?;
            (!matches!(outline, Outline::Point(_))).then_some(SectionCurve {
                face,
                source: ProjectionSource::Section {
                    body,
                    edge: reference,
                },
                outline,
            })
        })
        .collect();
    if curves.is_empty() {
        return Err(SectionError::Misses);
    }
    Ok(curves)
}

pub fn datum_outline(datum: &Plane, plane: &Plane, reach: f64) -> Option<Outline> {
    let normal = datum.normal();
    let gradient = Vector2::new(plane.x_axis().dot(normal), plane.y_axis().dot(normal));
    let steepness = gradient.length();
    if steepness <= DIRECTION_TOLERANCE || !reach.is_finite() || reach <= 0.0 {
        return None;
    }
    let foot =
        Point2::ZERO - gradient * (datum.signed_distance(plane.origin()) / steepness.powi(2));
    let along = gradient.perp() / steepness;
    Some(Outline::Line {
        start: foot - along * reach,
        end: foot + along * reach,
    })
}
