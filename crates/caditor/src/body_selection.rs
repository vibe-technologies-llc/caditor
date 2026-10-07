use std::collections::{BTreeMap, BTreeSet};

use caditor_document::{FeatureId, SolidResult};
use caditor_kernel::{EdgeId, Solid, tangent_chain, tangent_faces};

use crate::{
    bodies::{self, FaceKey},
    model::Model,
    selection::{Pickable, Selection, SelectionFilter},
    visibility,
};

pub const IN_SKETCH: &str = "Finish the sketch first; faces, edges and vertices of bodies are not selectable while one is edited";
pub const NO_KIND_TO_SELECT: &str =
    "Choose faces, edges or vertices in the selection filter first, or select one of them";
pub const NO_SHAPES: &str = "No shown body has anything of that kind to select";
pub const NO_EDGE_SELECTED: &str = "Select an edge first to follow its tangent edges";
pub const NO_TANGENT_EDGES: &str = "The selected edges have no tangent edge beyond themselves";
pub const NO_TANGENT_FACE_SELECTED: &str =
    "Select a face first to spread the selection to its tangent faces";
pub const NO_TANGENT_FACES: &str = "The selected faces meet no other face smoothly";
pub const NO_FACE_SELECTED: &str = "Select a face first to select the edges around it";
pub const NO_FACE_EDGES: &str = "The selected faces have no edge to select";

pub fn outside_sketch<T>(
    in_sketch: bool,
    offer: Result<T, &'static str>,
) -> Result<T, &'static str> {
    if in_sketch { Err(IN_SKETCH) } else { offer }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Faces,
    Edges,
    Vertices,
}

impl Kind {
    fn of(pickable: Pickable) -> Option<Self> {
        match pickable {
            Pickable::Face { .. } => Some(Self::Faces),
            Pickable::Edge { .. } => Some(Self::Edges),
            Pickable::Vertex { .. } => Some(Self::Vertices),
            _ => None,
        }
    }

    pub fn chosen(filter: SelectionFilter, selection: &Selection) -> Option<Self> {
        match filter {
            SelectionFilter::Faces => Some(Self::Faces),
            SelectionFilter::Edges => Some(Self::Edges),
            SelectionFilter::Vertices => Some(Self::Vertices),
            SelectionFilter::SketchGeometry => None,
            SelectionFilter::Everything => {
                let mut kinds = selection.iter().map(Self::of);
                let first = kinds.next()??;
                kinds.all(|kind| kind == Some(first)).then_some(first)
            }
        }
    }
}

fn shown_bodies(model: &Model) -> impl Iterator<Item = (FeatureId, &SolidResult)> {
    let document = model.document();
    let evaluation = model.evaluation();
    evaluation
        .bodies()
        .map(|(body, _)| body)
        .filter(|body| visibility::is_shown(document, *body))
        .filter_map(|body| Some((body, bodies::shown(evaluation, body)?)))
}

fn pickable_edges(
    body: FeatureId,
    result: &SolidResult,
    edges: impl IntoIterator<Item = EdgeId>,
) -> Vec<Pickable> {
    edges
        .into_iter()
        .filter(|edge| !bodies::is_seam(&result.solid, *edge))
        .filter_map(|edge| result.solid.edge(edge))
        .map(|edge| Pickable::Edge {
            body,
            edge: edge.name(),
        })
        .collect()
}

pub fn offer_select_all(
    model: &Model,
    selection: &Selection,
    filter: SelectionFilter,
) -> Result<Kind, &'static str> {
    let kind = Kind::chosen(filter, selection).ok_or(NO_KIND_TO_SELECT)?;
    if shown_bodies(model).next().is_none() {
        return Err(NO_SHAPES);
    }
    Ok(kind)
}

pub fn select_all(model: &Model, kind: Kind) -> Vec<Pickable> {
    shown_bodies(model)
        .flat_map(|(body, result)| match kind {
            Kind::Faces => bodies::face_keys(&result.solid)
                .into_iter()
                .map(|(_, face)| Pickable::Face { body, face })
                .collect::<Vec<_>>(),
            Kind::Edges => pickable_edges(body, result, result.solid.edges().map(|(id, _)| id)),
            Kind::Vertices => bodies::vertex_keys(result)
                .into_iter()
                .map(|(_, vertex)| Pickable::Vertex { body, vertex })
                .collect(),
        })
        .collect()
}

fn selected_edges(model: &Model, selection: &Selection) -> Vec<(FeatureId, EdgeId)> {
    selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Edge { body, edge } => {
                let result = bodies::shown(model.evaluation(), body)?;
                Some((body, bodies::find_edge(result, edge)?))
            }
            _ => None,
        })
        .collect()
}

pub fn offer_tangent_edges(selection: &Selection) -> Result<(), &'static str> {
    selection
        .iter()
        .any(|pickable| Kind::of(pickable) == Some(Kind::Edges))
        .then_some(())
        .ok_or(NO_EDGE_SELECTED)
}

pub fn tangent_edges(model: &Model, selection: &Selection) -> Vec<Pickable> {
    let selected = selected_edges(model, selection);
    let bodies: BTreeSet<FeatureId> = selected.iter().map(|(body, _)| *body).collect();
    bodies
        .into_iter()
        .filter_map(|body| Some((body, bodies::shown(model.evaluation(), body)?)))
        .flat_map(|(body, result)| {
            let starts: Vec<EdgeId> = selected
                .iter()
                .filter(|(owner, _)| *owner == body)
                .map(|(_, edge)| *edge)
                .collect();
            pickable_edges(body, result, tangent_chain(&result.solid, &starts))
        })
        .filter(|pickable| !selection.contains(*pickable))
        .collect()
}

fn selected_faces(selection: &Selection) -> Vec<(FeatureId, FaceKey)> {
    selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Face { body, face } => Some((body, face)),
            _ => None,
        })
        .collect()
}

pub fn offer_tangent_faces(selection: &Selection) -> Result<(), &'static str> {
    selection
        .iter()
        .any(|pickable| Kind::of(pickable) == Some(Kind::Faces))
        .then_some(())
        .ok_or(NO_TANGENT_FACE_SELECTED)
}

pub fn tangent_faces_of(model: &Model, selection: &Selection) -> Vec<Pickable> {
    let selected = selected_faces(selection);
    let bodies: BTreeSet<FeatureId> = selected.iter().map(|(body, _)| *body).collect();
    bodies
        .into_iter()
        .filter_map(|body| Some((body, bodies::shown(model.evaluation(), body)?)))
        .flat_map(|(body, result)| {
            let starts: Vec<caditor_kernel::FaceId> = selected
                .iter()
                .filter(|(owner, _)| *owner == body)
                .filter_map(|(_, key)| bodies::find_face(result, *key))
                .collect();
            let keys: BTreeMap<caditor_kernel::FaceId, FaceKey> =
                bodies::face_keys(&result.solid).into_iter().collect();
            tangent_faces(&result.solid, &starts)
                .into_iter()
                .filter_map(|face| {
                    let key = *keys.get(&face)?;
                    Some(Pickable::Face { body, face: key })
                })
                .collect::<Vec<_>>()
        })
        .filter(|pickable| !selection.contains(*pickable))
        .collect()
}

pub fn face_boundary(solid: &Solid, face: caditor_kernel::FaceId) -> Vec<EdgeId> {
    let Some(face) = solid.face(face) else {
        return Vec::new();
    };
    face.loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|face_loop| face_loop.coedges().iter())
        .filter_map(|id| solid.coedge(*id))
        .map(|coedge| coedge.edge())
        .collect()
}

pub fn offer_face_edges(selection: &Selection) -> Result<(), &'static str> {
    selection
        .iter()
        .any(|pickable| Kind::of(pickable) == Some(Kind::Faces))
        .then_some(())
        .ok_or(NO_FACE_SELECTED)
}

pub fn face_edges(model: &Model, selection: &Selection) -> Vec<Pickable> {
    selected_faces(selection)
        .into_iter()
        .filter_map(|(body, key)| {
            let result = bodies::shown(model.evaluation(), body)?;
            let face = bodies::find_face(result, key)?;
            Some(pickable_edges(
                body,
                result,
                face_boundary(&result.solid, face),
            ))
        })
        .flatten()
        .collect()
}
