use std::collections::BTreeMap;

use crate::{
    boolean::{
        BooleanError, FaceKey, Input, imprint::Arrangement, select::KeptFace, trace::Coedge,
    },
    build::plan::{Plan, PlanCoedge, PlanFace},
    topology::Solid,
};

fn unchanged_face(input: &Input, face: &KeptFace) -> bool {
    input.carrying
        && input.face(face.key).is_some_and(|original| {
            original.sense() == face.sense && *original.surface() == face.surface
        })
}

fn unchanged_coedge(
    input: &Input,
    arrangement: &Arrangement,
    key: FaceKey,
    coedge: &Coedge,
) -> bool {
    let piece = coedge.half_edge.piece;
    let Some(source) = arrangement
        .piece(piece)
        .and_then(|data| arrangement.source(data.source))
    else {
        return false;
    };
    let Some((operand, edge)) = source.edge else {
        return false;
    };
    if operand != key.operand || arrangement.edge_pieces(operand, edge) != [piece] {
        return false;
    }
    let solid = input.solid(operand);
    solid
        .edge(edge)
        .into_iter()
        .flat_map(|edge| edge.coedges())
        .filter(|id| solid.coedge_face(**id) == Some(key.face))
        .filter_map(|id| solid.coedge(*id))
        .any(|original| {
            original.sense() == coedge.half_edge.sense && *original.pcurve() == coedge.pcurve
        })
}

pub(super) fn assemble(
    input: &Input,
    arrangement: &Arrangement,
    faces: Vec<KeptFace>,
) -> Result<Solid, BooleanError> {
    let mut plan = Plan::default();
    let mut vertices: BTreeMap<usize, usize> = BTreeMap::new();
    let mut edges: BTreeMap<usize, usize> = BTreeMap::new();
    for face in faces {
        let unchanged = unchanged_face(input, &face);
        let mut loops = Vec::with_capacity(face.fragment.loops.len());
        for traced in face.fragment.loops {
            let mut coedges = Vec::with_capacity(traced.coedges.len());
            for coedge in traced.coedges {
                let settled = unchanged && unchanged_coedge(input, arrangement, face.key, &coedge);
                let piece = coedge.half_edge.piece;
                let edge = match edges.get(&piece) {
                    Some(edge) => *edge,
                    None => {
                        let (curve, data) = arrangement
                            .curve(piece)
                            .ok_or_else(|| BooleanError::Open(Box::default()))?;
                        let name = arrangement
                            .source(data.source)
                            .map(|source| source.name)
                            .unwrap_or_default();
                        let mut vertex = |index: usize| -> Result<usize, BooleanError> {
                            if let Some(vertex) = vertices.get(&index) {
                                return Ok(*vertex);
                            }
                            let point = arrangement
                                .point(index)
                                .ok_or_else(|| BooleanError::Open(Box::default()))?;
                            let vertex = plan.vertex(point);
                            vertices.insert(index, vertex);
                            Ok(vertex)
                        };
                        let ends = (vertex(data.start)?, vertex(data.end)?);
                        let edge = plan.edge(curve.clone(), data.interval, ends, name);
                        edges.insert(piece, edge);
                        edge
                    }
                };
                let sense = coedge.half_edge.sense;
                coedges.push(if settled {
                    PlanCoedge::settled(edge, sense, coedge.pcurve)
                } else {
                    PlanCoedge::given(edge, sense, coedge.pcurve)
                });
            }
            loops.push(coedges);
        }
        plan.face(PlanFace {
            surface: face.surface,
            sense: face.sense,
            name: face.name,
            origin: face.origin,
            loops,
        });
    }
    Ok(plan.build()?)
}
