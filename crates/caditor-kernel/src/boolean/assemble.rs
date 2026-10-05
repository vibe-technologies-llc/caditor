use std::collections::BTreeMap;

use crate::{
    boolean::{BooleanError, imprint::Arrangement, select::KeptFace},
    build::plan::{Plan, PlanCoedge, PlanFace},
    topology::Solid,
};

pub(super) fn assemble(
    arrangement: &Arrangement,
    faces: Vec<KeptFace>,
) -> Result<Solid, BooleanError> {
    let mut plan = Plan::default();
    let mut vertices: BTreeMap<usize, usize> = BTreeMap::new();
    let mut edges: BTreeMap<usize, usize> = BTreeMap::new();
    for face in faces {
        let mut loops = Vec::with_capacity(face.fragment.loops.len());
        for traced in face.fragment.loops {
            let mut coedges = Vec::with_capacity(traced.coedges.len());
            for coedge in traced.coedges {
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
                coedges.push(PlanCoedge::given(
                    edge,
                    coedge.half_edge.sense,
                    coedge.pcurve,
                ));
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
