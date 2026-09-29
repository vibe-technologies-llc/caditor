use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::{Point2, Point3, Vector3};
use thiserror::Error;

use crate::{
    build::SweepError,
    curve::{Curve, Line},
    error::GeometryError,
    interrupt,
    interval::Interval,
    naming::{EdgeName, FaceName, FaceOrigin, VertexName, occurrence_order},
    sense::Sense,
    surface::Surface,
    tolerance::{LINEAR_RESOLUTION, PCURVE_TOLERANCE},
    topology::{
        BuildError, EdgeId, FaceId, Pcurve, PcurveError, PcurveSample, Solid, SolidBuilder,
        fit_pcurve,
    },
};

#[derive(Debug, Clone, PartialEq, Error)]
pub(crate) enum PlanError {
    #[error("the planned solid refers to a vertex, edge or face it does not contain")]
    Unassembled,
    #[error(transparent)]
    Build(#[from] BuildError),
}

impl From<GeometryError> for PlanError {
    fn from(error: GeometryError) -> Self {
        Self::Build(BuildError::Geometry(error))
    }
}

impl From<PlanError> for SweepError {
    fn from(error: PlanError) -> Self {
        match error {
            PlanError::Unassembled => Self::Unassembled,
            PlanError::Build(error) => error.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanEdge {
    pub curve: Curve,
    pub interval: Interval,
    pub start: usize,
    pub end: usize,
    pub name: EdgeName,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlanPcurve {
    Fitted,
    Straight(Point2, Point2),
    Given(Pcurve),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanCoedge {
    pub edge: usize,
    pub sense: Sense,
    pub pcurve: PlanPcurve,
}

impl PlanCoedge {
    pub fn new(edge: usize, sense: Sense) -> Self {
        Self {
            edge,
            sense,
            pcurve: PlanPcurve::Fitted,
        }
    }

    pub fn mapped(edge: usize, sense: Sense, at_start: Point2, at_end: Point2) -> Self {
        Self {
            edge,
            sense,
            pcurve: PlanPcurve::Straight(at_start, at_end),
        }
    }

    pub fn given(edge: usize, sense: Sense, pcurve: Pcurve) -> Self {
        Self {
            edge,
            sense,
            pcurve: PlanPcurve::Given(pcurve),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanFace {
    pub surface: Surface,
    pub sense: Sense,
    pub name: FaceName,
    pub origin: Option<FaceOrigin>,
    pub loops: Vec<Vec<PlanCoedge>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Plan {
    vertices: Vec<Point3>,
    edges: Vec<PlanEdge>,
    faces: Vec<PlanFace>,
}

impl Plan {
    pub fn vertex(&mut self, point: Point3) -> usize {
        self.vertices.push(point);
        self.vertices.len() - 1
    }

    pub fn point(&self, vertex: usize) -> Option<Point3> {
        self.vertices.get(vertex).copied()
    }

    pub fn edge(
        &mut self,
        curve: Curve,
        interval: Interval,
        (start, end): (usize, usize),
        name: EdgeName,
    ) -> usize {
        self.edges.push(PlanEdge {
            curve,
            interval,
            start,
            end,
            name,
        });
        self.edges.len() - 1
    }

    pub fn line(&mut self, start: usize, end: usize, name: EdgeName) -> Result<usize, SweepError> {
        let (Some(from), Some(to)) = (self.point(start), self.point(end)) else {
            return Err(SweepError::Unassembled);
        };
        let line = Line::through(from, to)?;
        let interval = Interval::new(0.0, from.distance(to)).ok_or(SweepError::Unassembled)?;
        Ok(self.edge(line.into(), interval, (start, end), name))
    }

    pub fn face(&mut self, face: PlanFace) {
        self.faces.push(face);
    }

    fn users(&self) -> Vec<Vec<(usize, Sense)>> {
        let mut users = vec![Vec::new(); self.edges.len()];
        for (index, face) in self.faces.iter().enumerate() {
            for coedge in face.loops.iter().flatten() {
                if let Some(list) = users.get_mut(coedge.edge) {
                    list.push((index, coedge.sense));
                }
            }
        }
        users
    }

    fn disambiguate(&mut self) {
        let users = self.users();
        let mut around: Vec<BTreeSet<FaceName>> = vec![BTreeSet::new(); self.vertices.len()];
        for (edge, list) in self.edges.iter().zip(&users) {
            for (face, _) in list {
                let Some(face) = self.faces.get(*face) else {
                    continue;
                };
                for vertex in [edge.start, edge.end] {
                    if let Some(set) = around.get_mut(vertex) {
                        set.insert(face.name);
                    }
                }
            }
        }
        let vertex_names: Vec<VertexName> = around.into_iter().map(VertexName::of_faces).collect();
        let mut groups: BTreeMap<EdgeName, Vec<usize>> = BTreeMap::new();
        for (index, edge) in self.edges.iter().enumerate() {
            groups.entry(edge.name).or_default().push(index);
        }
        let face_name = |face: Option<&(usize, Sense)>| {
            face.and_then(|(face, _)| self.faces.get(*face))
                .map_or(FaceName::NONE, |face| face.name)
        };
        let mut renamed: Vec<(usize, EdgeName)> = Vec::new();
        for members in groups.values().filter(|members| members.len() > 1) {
            let mut named: Vec<(usize, EdgeName)> = members
                .iter()
                .filter_map(|index| {
                    let edge = self.edges.get(*index)?;
                    let list = users.get(*index)?;
                    let left = face_name(list.iter().find(|(_, sense)| sense.is_same()));
                    let right = face_name(list.iter().find(|(_, sense)| !sense.is_same()));
                    let from = vertex_names.get(edge.start).copied().unwrap_or_default();
                    let to = vertex_names.get(edge.end).copied().unwrap_or_default();
                    Some((*index, EdgeName::between_at(left, right, from, to)))
                })
                .collect();
            let mut counts: BTreeMap<EdgeName, usize> = BTreeMap::new();
            for (_, name) in &named {
                *counts.entry(*name).or_default() += 1;
            }
            let midpoint = |index: usize| {
                self.edges.get(index).map_or(Vector3::ZERO, |edge| {
                    edge.curve.point(edge.interval.middle())
                })
            };
            named.sort_by_key(|(index, _)| occurrence_order(midpoint(*index)));
            let mut occurrences: BTreeMap<EdgeName, u32> = BTreeMap::new();
            for (index, name) in named {
                if counts.get(&name).copied().unwrap_or(0) > 1 {
                    let occurrence = occurrences.entry(name).or_insert(0);
                    renamed.push((index, EdgeName::occurrence(name, *occurrence)));
                    *occurrence += 1;
                } else {
                    renamed.push((index, name));
                }
            }
        }
        for (index, name) in renamed {
            if let Some(edge) = self.edges.get_mut(index) {
                edge.name = name;
            }
        }
    }

    fn shells(&self) -> Vec<Vec<usize>> {
        let mut root: Vec<usize> = (0..self.faces.len()).collect();
        let find = |root: &[usize], mut item: usize| {
            while let Some(parent) = root.get(item).copied() {
                if parent == item {
                    break;
                }
                item = parent;
            }
            item
        };
        for list in self.users() {
            let mut faces = list.iter().map(|(face, _)| *face);
            let Some(first) = faces.next() else {
                continue;
            };
            for other in faces {
                let (a, b) = (find(&root, first), find(&root, other));
                let (low, high) = (a.min(b), a.max(b));
                if let Some(slot) = root.get_mut(high) {
                    *slot = low;
                }
            }
        }
        let mut shells: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for face in 0..self.faces.len() {
            shells.entry(find(&root, face)).or_default().push(face);
        }
        shells.into_values().collect()
    }

    fn merge_coincident_vertices(&mut self) {
        let users = self.users();
        let mut representative: Vec<usize> = (0..self.vertices.len()).collect();
        for faces in self.shells() {
            let faces: BTreeSet<usize> = faces.into_iter().collect();
            let mut members: Vec<usize> = self
                .edges
                .iter()
                .zip(&users)
                .filter(|(_, list)| list.iter().any(|(face, _)| faces.contains(face)))
                .flat_map(|(edge, _)| [edge.start, edge.end])
                .collect();
            members.sort_unstable();
            members.dedup();
            let mut by_x: Vec<(Point3, usize)> = members
                .iter()
                .filter_map(|vertex| Some((self.point(*vertex)?, *vertex)))
                .collect();
            by_x.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.1.cmp(&b.1)));
            for (position, (point, vertex)) in by_x.iter().enumerate() {
                let nearby = by_x
                    .iter()
                    .skip(position + 1)
                    .take_while(|(other, _)| other.x - point.x <= LINEAR_RESOLUTION)
                    .chain(
                        by_x.iter()
                            .take(position)
                            .rev()
                            .take_while(|(other, _)| point.x - other.x <= LINEAR_RESOLUTION),
                    );
                let earlier = nearby
                    .filter(|(other, id)| {
                        *id < *vertex && other.distance(*point) <= LINEAR_RESOLUTION
                    })
                    .map(|(_, id)| *id)
                    .min();
                if let (Some(earlier), Some(slot)) = (earlier, representative.get_mut(*vertex)) {
                    *slot = earlier;
                }
            }
        }
        for edge in &mut self.edges {
            edge.start = representative
                .get(edge.start)
                .copied()
                .unwrap_or(edge.start);
            edge.end = representative.get(edge.end).copied().unwrap_or(edge.end);
        }
    }

    pub fn build(mut self) -> Result<Solid, PlanError> {
        self.merge_coincident_vertices();
        self.disambiguate();
        let mut builder = SolidBuilder::new();
        let used: BTreeSet<usize> = self
            .edges
            .iter()
            .flat_map(|edge| [edge.start, edge.end])
            .collect();
        let mut vertices = BTreeMap::new();
        for index in used {
            let point = self
                .vertices
                .get(index)
                .copied()
                .ok_or(PlanError::Unassembled)?;
            vertices.insert(index, builder.vertex(point)?);
        }
        let vertex = |index: usize| vertices.get(&index).copied().ok_or(PlanError::Unassembled);
        let mut edges: Vec<EdgeId> = Vec::with_capacity(self.edges.len());
        for edge in &self.edges {
            let id = builder.edge(
                edge.curve.clone(),
                edge.interval,
                vertex(edge.start)?,
                vertex(edge.end)?,
            )?;
            builder.set_edge_name(id, edge.name)?;
            edges.push(id);
        }
        for shell_faces in self.shells() {
            let shell = builder.shell()?;
            for index in shell_faces {
                interrupt::check().map_err(BuildError::from)?;
                let Some(face) = self.faces.get(index) else {
                    continue;
                };
                let id = builder.face(shell, face.surface.clone(), face.sense)?;
                builder.set_face_name(id, face.name)?;
                if let Some(origin) = face.origin {
                    builder.set_face_origin(id, origin)?;
                }
                for coedges in &face.loops {
                    self.add_loop(&mut builder, id, face, coedges, &edges)?;
                }
            }
        }
        Ok(builder.build()?)
    }

    fn add_loop(
        &self,
        builder: &mut SolidBuilder,
        face_id: FaceId,
        face: &PlanFace,
        coedges: &[PlanCoedge],
        edges: &[EdgeId],
    ) -> Result<(), PlanError> {
        let resolved = coedges
            .iter()
            .map(|coedge| {
                edges
                    .get(coedge.edge)
                    .copied()
                    .map(|id| (id, coedge))
                    .ok_or(PlanError::Unassembled)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let given = resolved
            .iter()
            .any(|(_, coedge)| matches!(coedge.pcurve, PlanPcurve::Given(_)));
        let straight = resolved
            .iter()
            .all(|(_, coedge)| matches!(coedge.pcurve, PlanPcurve::Straight(..)));
        if !given && !straight {
            let plain: Vec<(EdgeId, Sense)> = resolved
                .iter()
                .map(|(id, coedge)| (*id, coedge.sense))
                .collect();
            builder.add_loop(face_id, &plain)?;
            return Ok(());
        }
        let mut with_pcurves: Vec<(EdgeId, Sense, Pcurve)> = Vec::with_capacity(resolved.len());
        for (id, coedge) in resolved {
            let edge = self.edges.get(coedge.edge).ok_or(PlanError::Unassembled)?;
            let hint = with_pcurves.last().map(|(_, _, pcurve)| pcurve.end());
            let pcurve = match &coedge.pcurve {
                PlanPcurve::Given(pcurve) => Ok(pcurve.clone()),
                PlanPcurve::Straight(at_start, at_end) => {
                    straight_pcurve(edge.interval, coedge.sense, *at_start, *at_end)
                }
                PlanPcurve::Fitted => fit_pcurve(
                    &face.surface,
                    &edge.curve,
                    edge.interval,
                    coedge.sense,
                    hint,
                ),
            }
            .map_err(|error| BuildError::Pcurve { edge: id, error })?;
            with_pcurves.push((id, coedge.sense, pcurve));
        }
        builder.add_loop_with_pcurves(face_id, with_pcurves)?;
        Ok(())
    }
}

pub(super) fn outward_sense(
    surface: &Surface,
    point: Point3,
    outward: Vector3,
    near: Option<Point2>,
) -> Sense {
    let uv = surface.project(point, near);
    let normal = surface.normal(uv.x, uv.y).unwrap_or(Vector3::ZERO);
    Sense::from_sign(normal.dot(outward))
}

fn straight_pcurve(
    interval: Interval,
    sense: Sense,
    at_start: Point2,
    at_end: Point2,
) -> Result<Pcurve, PcurveError> {
    let first = PcurveSample {
        parameter: interval.start(),
        uv: at_start,
    };
    let last = PcurveSample {
        parameter: interval.end(),
        uv: at_end,
    };
    let samples = if sense.is_same() {
        vec![first, last]
    } else {
        vec![last, first]
    };
    Pcurve::new(samples, PCURVE_TOLERANCE)
}
