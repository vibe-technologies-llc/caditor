mod geometry;
mod graph;
mod spline;
mod structure;
#[cfg(test)]
mod tests;
mod topology;
mod units;

use caditor_geometry::RigidTransform;
use caditor_kernel::Solid;

use crate::{
    part21::{SyntaxError, parse},
    read::{
        geometry::Geometry,
        graph::{Entity, Graph, Problem},
        structure::{MAX_INSTANCES as MAX_PLACEMENTS, Structure},
        topology::Topology,
        units::Units,
    },
};

const SOLID_KINDS: [&str; 2] = ["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS"];

#[derive(Debug, Clone, PartialEq)]
pub struct StepSolid {
    pub name: String,
    pub solid: Solid,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepModel {
    pub solids: Vec<StepSolid>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("it is not a STEP file")]
    NotStep,
    #[error("the file is damaged near line {0}")]
    Damaged(usize),
    #[error("it holds no solid bodies; caditor imports closed solids only")]
    NoSolids,
    #[error("{0}")]
    Unreadable(String),
}

pub fn read_step(text: &str) -> Result<StepModel, ReadError> {
    let exchange = parse(text).map_err(|error| match error {
        SyntaxError::NotStep => ReadError::NotStep,
        SyntaxError::Damaged { line } => ReadError::Damaged(line),
    })?;
    let graph = Graph::new(&exchange);
    let mut structure = Structure::read(&graph);
    let mut model = StepModel::default();
    let mut failures = Vec::new();
    let mut unnamed_units = false;
    let mut repaired = 0;
    let solids: Vec<Entity<'_>> = graph
        .entities()
        .filter(|entity| SOLID_KINDS.contains(&entity.kind()))
        .collect();
    let skipped = graph
        .entities()
        .filter(|entity| matches!(entity.kind(), "SHELL_BASED_SURFACE_MODEL" | "FACETED_BREP"))
        .count();
    for (index, entity) in solids.iter().enumerate() {
        let representation = structure.representation_of(entity.id);
        let units = representation
            .map(|representation| structure.units_of(&graph, representation))
            .unwrap_or_default();
        unnamed_units |= !units.named;
        let name = solid_name(entity, &structure, representation, index);
        let placements = match representation {
            Some(representation) => structure.placements(representation),
            None => vec![RigidTransform::IDENTITY],
        };
        match build(&graph, units, entity.id) {
            Ok((solid, healed)) => {
                repaired += healed;
                let count = placements.len();
                for (instance, placement) in placements.into_iter().enumerate() {
                    let placed = if placement == RigidTransform::IDENTITY {
                        Ok(solid.clone())
                    } else {
                        solid.transformed(&placement)
                    };
                    let Ok(placed) = placed else {
                        continue;
                    };
                    let name = if count > 1 && instance > 0 {
                        format!("{name} {}", instance + 1)
                    } else {
                        name.clone()
                    };
                    model.solids.push(StepSolid {
                        name,
                        solid: placed,
                    });
                }
            }
            Err(problem) => {
                log::warn!("could not import solid #{}: {problem}", entity.id);
                failures.push((name, problem));
            }
        }
    }
    for (name, problem) in &failures {
        model.notes.push(format!(
            "“{name}” could not be imported, because its entity {problem}."
        ));
    }
    if structure.truncated {
        model.notes.push(format!(
            "The assembly places some parts more than {MAX_PLACEMENTS} times; only the first \
             {MAX_PLACEMENTS} copies were imported."
        ));
    }
    if skipped > 0 {
        model.notes.push(format!(
            "{} that {} not closed solids {} left out.",
            if skipped == 1 {
                "1 surface body".to_owned()
            } else {
                format!("{skipped} surface bodies")
            },
            if skipped == 1 { "is" } else { "are" },
            if skipped == 1 { "was" } else { "were" }
        ));
    }
    if repaired > 0 {
        model.notes.push(format!(
            "{} that did not quite meet {} moved onto {} faces.",
            if repaired == 1 {
                "1 edge or corner".to_owned()
            } else {
                format!("{repaired} edges and corners")
            },
            if repaired == 1 {
                "its faces was"
            } else {
                "their faces were"
            },
            if repaired == 1 { "its" } else { "their" }
        ));
    }
    if unnamed_units && !model.solids.is_empty() {
        model.notes.push(
            "The file does not say which unit it uses, so its numbers were read as millimetres."
                .to_owned(),
        );
    }
    if model.solids.is_empty() {
        return Err(match failures.into_iter().next() {
            Some((name, problem)) => ReadError::Unreadable(format!(
                "“{name}” could not be rebuilt, because its entity {problem}"
            )),
            None => ReadError::NoSolids,
        });
    }
    Ok(model)
}

fn build(graph: &Graph<'_>, units: Units, id: u64) -> Result<(Solid, usize), Problem> {
    let geometry = Geometry {
        graph: *graph,
        units,
    };
    Topology::new(&geometry).solid(id)
}

fn solid_name(
    entity: &Entity<'_>,
    structure: &Structure,
    representation: Option<u64>,
    index: usize,
) -> String {
    let own = entity
        .fields()
        .map(|fields| fields.text(0).trim().to_owned())
        .unwrap_or_default();
    if !own.is_empty() {
        return own;
    }
    representation
        .and_then(|representation| structure.name_of(representation))
        .map_or_else(|| format!("Body {}", index + 1), str::to_owned)
}
