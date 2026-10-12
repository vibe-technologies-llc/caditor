use std::collections::BTreeMap;

use caditor_geometry::{Point3, Vector3};
use caditor_kernel::{Curve, Edge, EdgeId, Interval, Solid, VertexId};

const LENGTH_BISECTIONS: usize = 50;

pub(crate) fn parameter_at_length(curve: &Curve, interval: Interval, along: f64) -> f64 {
    match curve {
        Curve::Line(_) => interval.start() + along,
        Curve::Circle(circle) => interval.start() + along / circle.radius(),
        _ => {
            let (mut low, mut high) = (interval.start(), interval.end());
            for _ in 0..LENGTH_BISECTIONS {
                let middle = 0.5 * (low + high);
                let reached = Interval::new(interval.start(), middle)
                    .map_or(0.0, |range| curve.length(range));
                if reached < along {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            0.5 * (low + high)
        }
    }
}

struct Traversed<'a> {
    id: EdgeId,
    edge: &'a Edge,
    forward: bool,
    length: f64,
}

impl<'a> Traversed<'a> {
    fn new(id: EdgeId, edge: &'a Edge, forward: bool) -> Self {
        Self {
            id,
            edge,
            forward,
            length: edge.curve().length(edge.interval()),
        }
    }

    fn reversed(self) -> Self {
        Self {
            forward: !self.forward,
            ..self
        }
    }

    fn at(&self, along: f64) -> (Point3, Vector3) {
        let curve = self.edge.curve();
        let interval = self.edge.interval();
        let from_start = if self.forward {
            along
        } else {
            self.length - along
        };
        let parameter = interval.clamp(parameter_at_length(curve, interval, from_start.max(0.0)));
        let derivatives = curve.evaluate(parameter);
        let tangent = if self.forward {
            derivatives.first
        } else {
            -derivatives.first
        };
        (derivatives.point, tangent)
    }
}

pub(crate) struct EdgeRun<'a> {
    first: Traversed<'a>,
    rest: Vec<Traversed<'a>>,
}

impl<'a> EdgeRun<'a> {
    pub(crate) fn single(id: EdgeId, edge: &'a Edge) -> Self {
        Self {
            first: Traversed::new(id, edge, true),
            rest: Vec::new(),
        }
    }

    pub(crate) fn chained(
        solid: &'a Solid,
        pieces: &[EdgeId],
        leading: Option<EdgeId>,
    ) -> Option<Self> {
        let edges: Vec<(EdgeId, &Edge)> = pieces
            .iter()
            .map(|id| solid.edge(*id).map(|edge| (*id, edge)))
            .collect::<Option<_>>()?;
        if edges.iter().any(|(_, edge)| edge.is_closed()) {
            return None;
        }

        let mut touching: BTreeMap<VertexId, usize> = BTreeMap::new();
        for (_, edge) in &edges {
            *touching.entry(edge.start()).or_default() += 1;
            *touching.entry(edge.end()).or_default() += 1;
        }
        let ends: Vec<VertexId> = touching
            .iter()
            .filter(|(_, count)| **count == 1)
            .map(|(vertex, _)| *vertex)
            .collect();
        let branched = touching.values().any(|count| *count > 2);
        let [start, _] = ends.as_slice() else {
            return None;
        };
        if branched {
            return None;
        }

        let mut left = edges;
        let mut at = *start;
        let mut chain = Vec::with_capacity(left.len());
        while let Some(index) = left
            .iter()
            .position(|(_, edge)| edge.start() == at || edge.end() == at)
        {
            let (id, edge) = left.swap_remove(index);
            let forward = edge.start() == at;
            at = if forward { edge.end() } else { edge.start() };
            chain.push(Traversed::new(id, edge, forward));
        }
        if !left.is_empty() {
            return None;
        }

        let leading_backward = match leading {
            Some(leading) => chain
                .iter()
                .find(|piece| piece.id == leading)
                .is_some_and(|piece| !piece.forward),
            None => chain.first().is_some_and(|piece| !piece.forward),
        };
        if leading_backward {
            chain = chain.into_iter().rev().map(Traversed::reversed).collect();
        }
        let mut pieces = chain.into_iter();
        let first = pieces.next()?;
        Some(Self {
            first,
            rest: pieces.collect(),
        })
    }

    fn pieces(&self) -> impl Iterator<Item = &Traversed<'a>> {
        std::iter::once(&self.first).chain(&self.rest)
    }

    pub(crate) fn length(&self) -> f64 {
        self.pieces().map(|piece| piece.length).sum()
    }

    pub(crate) fn at(&self, along: f64) -> (Point3, Vector3) {
        let mut before = 0.0;
        for piece in self.pieces() {
            if along <= before + piece.length {
                return piece.at(along - before);
            }
            before += piece.length;
        }
        let last = self.rest.last().unwrap_or(&self.first);
        last.at(last.length)
    }
}
