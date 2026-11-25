use std::collections::{BTreeMap, BTreeSet};

use crate::{
    boolean::{
        BooleanError, FaceKey, Input, Planned, imprint::Arrangement, select::KeptFace,
        trace::Coedge,
    },
    build::plan::{Plan, PlanCoedge, PlanFace, Provenance},
    naming::EdgeName,
    topology::{CoedgeId, Solid},
};

fn keeps_side(input: &Input, face: &KeptFace) -> bool {
    input.carrying
        && input
            .face(face.key)
            .is_some_and(|original| original.sense() == face.sense)
}

fn unchanged_face(input: &Input, face: &KeptFace) -> bool {
    input.carrying
        && input.face(face.key).is_some_and(|original| {
            original.sense() == face.sense && *original.surface() == face.surface
        })
}

fn original_coedge(
    input: &Input,
    arrangement: &Arrangement,
    key: FaceKey,
    coedge: &Coedge,
) -> Option<CoedgeId> {
    let piece = coedge.half_edge.piece;
    let source = arrangement
        .piece(piece)
        .and_then(|data| arrangement.source(data.source))?;
    let (operand, edge) = source.edge?;

    if operand != key.operand || arrangement.edge_pieces(operand, edge) != [piece] {
        return None;
    }

    let solid = input.solid(operand);
    solid
        .edge(edge)?
        .coedges()
        .iter()
        .copied()
        .filter(|id| solid.coedge_face(*id) == Some(key.face))
        .find(|id| {
            solid.coedge(*id).is_some_and(|original| {
                original.sense() == coedge.half_edge.sense && *original.pcurve() == coedge.pcurve
            })
        })
}

fn whole_loop(solid: &Solid, originals: &[Option<CoedgeId>]) -> bool {
    let Some(Some(first)) = originals.first() else {
        return false;
    };
    let Some(face_loop) = solid
        .coedge(*first)
        .and_then(|coedge| solid.face_loop(coedge.owner()))
    else {
        return false;
    };
    let members = face_loop.coedges();

    let Some(start) = members.iter().position(|member| member == first) else {
        return false;
    };

    members.len() == originals.len()
        && originals.iter().enumerate().all(|(offset, original)| {
            members.get((start + offset) % members.len()).copied() == *original
        })
}

pub(super) fn assemble(
    input: &Input,
    arrangement: &Arrangement,
    faces: Vec<KeptFace>,
    renamed: &BTreeMap<usize, EdgeName>,
) -> Result<Solid, BooleanError> {
    assemble_planned(input, arrangement, faces, renamed).map(|planned| planned.solid)
}

pub(super) fn assemble_planned(
    input: &Input,
    arrangement: &Arrangement,
    faces: Vec<KeptFace>,
    renamed: &BTreeMap<usize, EdgeName>,
) -> Result<Planned, BooleanError> {
    let mut plan = Plan::default();
    let mut vertices: BTreeMap<usize, usize> = BTreeMap::new();
    let mut edges: BTreeMap<usize, usize> = BTreeMap::new();

    for face in faces {
        let unchanged = unchanged_face(input, &face);
        let mut provenance = Provenance {
            keeps_side: keeps_side(input, &face),
            carried_loops: BTreeSet::new(),
        };
        let mut loops = Vec::with_capacity(face.fragment.loops.len());

        for (position, traced) in face.fragment.loops.into_iter().enumerate() {
            let mut coedges = Vec::with_capacity(traced.coedges.len());
            let mut originals = Vec::with_capacity(traced.coedges.len());

            for coedge in traced.coedges {
                let original = unchanged
                    .then(|| original_coedge(input, arrangement, face.key, &coedge))
                    .flatten();
                let settled = original.is_some();

                originals.push(original);

                let piece = coedge.half_edge.piece;
                let edge = match edges.get(&piece) {
                    Some(edge) => *edge,
                    None => {
                        let (curve, data) = arrangement
                            .curve(piece)
                            .ok_or_else(|| BooleanError::Open(Box::default()))?;
                        let name = renamed.get(&piece).copied().unwrap_or_else(|| {
                            arrangement
                                .source(data.source)
                                .map(|source| source.name)
                                .unwrap_or_default()
                        });
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
            if whole_loop(input.solid(face.key.operand), &originals) {
                provenance.carried_loops.insert(position);
            }
            loops.push(coedges);
        }

        let planned = PlanFace {
            surface: face.surface,
            sense: face.sense,
            name: face.name,
            origin: face.origin,
            loops,
        };

        plan.face_from(planned, provenance);
    }

    let names = plan.edge_names().collect();

    Ok(Planned {
        solid: plan.build()?,
        names,
    })
}
