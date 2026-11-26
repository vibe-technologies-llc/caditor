use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Point2, Point3};
use caditor_kernel::{Domain, Interval, Surface, check_interrupt};

use crate::read::{geometry::Geometry, graph::Graph, offset::Region, topology::parameter_range};

const REGION_MARGIN: f64 = 0.1;
const REGION_FLOOR: f64 = 0.01;
const EDGE_SAMPLES: usize = 16;
const WRAPPERS: [&str; 3] = [
    "OFFSET_SURFACE",
    "SURFACE_REPLICA",
    "RECTANGULAR_TRIMMED_SURFACE",
];

#[derive(Debug, Default)]
pub(crate) struct Uses {
    faces: BTreeMap<u64, Vec<u64>>,
    wrapped: BTreeSet<u64>,
}

impl Uses {
    pub fn of(graph: Graph<'_>) -> Self {
        let mut uses = Self::default();

        for entity in graph.entities() {
            let kind = entity.kind();

            if matches!(kind, "ADVANCED_FACE" | "FACE_SURFACE") {
                if let Some(surface) = entity
                    .fields()
                    .ok()
                    .and_then(|fields| fields.optional_reference(2))
                {
                    uses.faces.entry(surface).or_default().push(entity.id);
                }
            } else if WRAPPERS.contains(&kind)
                && let Some(basis) = entity
                    .find(kind)
                    .and_then(|fields| fields.optional_reference(1))
            {
                uses.wrapped.insert(basis);
            }
        }
        uses
    }

    fn only_faces_on(&self, surface: u64) -> Option<&[u64]> {
        if self.wrapped.contains(&surface) {
            return None;
        }
        self.faces.get(&surface).map(Vec::as_slice)
    }
}

pub(crate) fn used_region(
    geometry: &Geometry<'_>,
    surface: u64,
    basis: &Surface,
) -> Option<Region> {
    let faces = geometry.uses().only_faces_on(surface)?;
    let mut low = Point2::splat(f64::INFINITY);
    let mut high = Point2::splat(f64::NEG_INFINITY);

    for face in faces {
        check_interrupt().ok()?;

        for run in face_runs(geometry, *face)? {
            let mut previous = None;

            for point in run {
                let uv = basis.project(point, previous);

                previous = Some(uv);
                low = low.min(uv);
                high = high.max(uv);
            }
        }
    }
    if !low.is_finite() || !high.is_finite() {
        return None;
    }
    Some(Region {
        u: widened(low.x, high.x, basis.u_domain())?,
        v: widened(low.y, high.y, basis.v_domain())?,
    })
}

fn widened(low: f64, high: f64, domain: Domain) -> Option<Interval> {
    let floor = domain
        .bounded()
        .map_or(0.0, |whole| REGION_FLOOR * whole.length());
    let margin = REGION_MARGIN * (high - low) + floor;

    Interval::new(domain.clamp(low - margin), domain.clamp(high + margin))
        .filter(|range| range.length() > 0.0)
}

fn face_runs(geometry: &Geometry<'_>, face: u64) -> Option<Vec<Vec<Point3>>> {
    let graph = geometry.graph;
    let mut runs = Vec::new();

    for bound in graph.entity(face).ok()?.fields().ok()?.references(1).ok()? {
        let loop_id = graph.entity(bound).ok()?.fields().ok()?.reference(1).ok()?;
        let loop_entity = graph.entity(loop_id).ok()?;
        let loop_fields = loop_entity.fields().ok()?;

        match loop_entity.kind() {
            "EDGE_LOOP" => {
                for oriented in loop_fields.references(1).ok()? {
                    let edge = graph
                        .entity(oriented)
                        .ok()?
                        .record("ORIENTED_EDGE")
                        .ok()?
                        .reference(3)
                        .ok()?;

                    runs.push(edge_samples(geometry, edge)?);
                }
            }
            "VERTEX_LOOP" => runs.push(vec![vertex(geometry, loop_fields.reference(1).ok()?)?]),
            "POLY_LOOP" => runs.push(
                loop_fields
                    .references(1)
                    .ok()?
                    .into_iter()
                    .map(|point| geometry.point(point).ok())
                    .collect::<Option<Vec<Point3>>>()?,
            ),
            _ => return None,
        }
    }
    Some(runs)
}

fn edge_samples(geometry: &Geometry<'_>, edge: u64) -> Option<Vec<Point3>> {
    geometry.charge(edge).ok()?;

    let fields = geometry
        .graph
        .entity(edge)
        .ok()?
        .record("EDGE_CURVE")
        .ok()?;
    let (first, second) = (fields.reference(1).ok()?, fields.reference(2).ok()?);

    let (start, end) = if fields.logical(4).ok()? {
        (first, second)
    } else {
        (second, first)
    };

    let ends = [vertex(geometry, start)?, vertex(geometry, end)?];
    let curve = geometry.curve(fields.reference(3).ok()?).ok()?;

    let Some(range) = parameter_range(&curve, ends[0], ends[1], start == end) else {
        return Some(ends.to_vec());
    };
    Some(
        range
            .split(EDGE_SAMPLES)
            .map(|parameter| curve.point(parameter))
            .chain(ends)
            .collect(),
    )
}

fn vertex(geometry: &Geometry<'_>, id: u64) -> Option<Point3> {
    let point = geometry
        .graph
        .entity(id)
        .ok()?
        .record("VERTEX_POINT")
        .ok()?
        .reference(1)
        .ok()?;
    geometry.point(point).ok()
}
