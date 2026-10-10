use std::collections::BTreeSet;

use super::{Topology, continues};
use crate::topology::{EdgeId, Solid, VertexId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Run {
    pub edge: EdgeId,
    pub from: f64,
    pub to: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Chain {
    pub runs: Vec<Run>,
    pub closed: bool,
    pub ends: [Option<VertexId>; 2],
}

impl Chain {
    pub fn edges(&self) -> BTreeSet<EdgeId> {
        self.runs.iter().map(|run| run.edge).collect()
    }
}

fn next_at(
    solid: &Solid,
    topology: &Topology,
    chosen: &BTreeSet<EdgeId>,
    edge: EdgeId,
    vertex: VertexId,
) -> Option<EdgeId> {
    topology
        .edges_at(vertex)
        .iter()
        .copied()
        .find(|other| chosen.contains(other) && continues(solid, edge, *other, vertex))
}

fn far_end(solid: &Solid, edge: EdgeId, near: VertexId) -> Option<VertexId> {
    let definition = solid.edge(edge)?;
    if definition.start() == near {
        Some(definition.end())
    } else {
        Some(definition.start())
    }
}

fn run_from(solid: &Solid, edge: EdgeId, entered: VertexId) -> Option<Run> {
    let definition = solid.edge(edge)?;
    let interval = definition.interval();
    Some(if definition.start() == entered {
        Run {
            edge,
            from: interval.start(),
            to: interval.end(),
        }
    } else {
        Run {
            edge,
            from: interval.end(),
            to: interval.start(),
        }
    })
}

fn split_closed(solid: &Solid, edge: EdgeId) -> Option<Chain> {
    let interval = solid.edge(edge)?.interval();
    let middle = interval.middle();
    Some(Chain {
        runs: vec![
            Run {
                edge,
                from: interval.start(),
                to: middle,
            },
            Run {
                edge,
                from: middle,
                to: interval.end(),
            },
        ],
        closed: true,
        ends: [None, None],
    })
}

fn walk(
    solid: &Solid,
    topology: &Topology,
    chosen: &BTreeSet<EdgeId>,
    seed: EdgeId,
) -> Option<Chain> {
    let definition = solid.edge(seed)?;
    if definition.is_closed() {
        return split_closed(solid, seed);
    }
    let mut first = seed;
    let mut entered = definition.start();
    let mut seen = BTreeSet::from([seed]);
    while let Some(before) = next_at(solid, topology, chosen, first, entered) {
        if !seen.insert(before) {
            break;
        }
        entered = far_end(solid, before, entered)?;
        first = before;
    }
    let start_vertex = entered;
    let mut runs = Vec::new();
    let mut edge = first;
    let mut visited = BTreeSet::new();
    loop {
        visited.insert(edge);
        runs.push(run_from(solid, edge, entered)?);
        let leaving = far_end(solid, edge, entered)?;
        match next_at(solid, topology, chosen, edge, leaving) {
            Some(next) if next == first && leaving == start_vertex => {
                return Some(Chain {
                    runs,
                    closed: true,
                    ends: [None, None],
                });
            }
            Some(next) if !visited.contains(&next) => {
                entered = leaving;
                edge = next;
            }
            _ => {
                return Some(Chain {
                    runs,
                    closed: false,
                    ends: [Some(start_vertex), Some(leaving)],
                });
            }
        }
    }
}

pub(super) fn chains(solid: &Solid, topology: &Topology, chosen: &BTreeSet<EdgeId>) -> Vec<Chain> {
    let mut left = chosen.clone();
    let mut found = Vec::new();
    while let Some(seed) = left.iter().next().copied() {
        let Some(chain) = walk(solid, topology, chosen, seed) else {
            left.remove(&seed);
            continue;
        };
        for edge in chain.edges() {
            left.remove(&edge);
        }
        left.remove(&seed);
        found.push(chain);
    }
    found
}
